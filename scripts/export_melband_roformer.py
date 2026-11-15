#!/usr/bin/env python3
"""Export KimberleyJensen's Mel-Band RoFormer vocal model to ONNX with the STFT outside the graph.

Weights: https://huggingface.co/KimberleyJSN/melbandroformer (model card licence: MIT).
Model code: https://github.com/KimberleyJensen/Mel-Band-Roformer-Vocal-Model (derived from
lucidrains/BS-RoFormer, MIT). Neither is vendored here; pass their paths.

Graph contract (AGENTS.md §4.2, engine-ml `roformer.rs` does the rest on the host):
  input  spec [1, 2*F, T, 2]  f32  torch.stft(n_fft=2048, hop=441, hann, center, unnormalized)
                                    of both channels, frequency-major with stereo interleaved
                                    (row 2f+c = bin f of channel c), last axis = (re, im)
  output mask [1, 2*F, T, 2]  f32  complex mask, already averaged over overlapping mel bands
  stem = istft(spec * mask) per channel.

Usage:
  python -I scripts/export_melband_roformer.py --repo <Mel-Band-Roformer-Vocal-Model> \
      --ckpt MelBandRoformer.ckpt --out models/weights/melband_roformer_vocals.onnx [--check clip.wav]
"""

import argparse
import json
import sys
from pathlib import Path

import torch
import yaml
from einops import pack, rearrange, unpack


class MaskNet(torch.nn.Module):
    """The model's forward pass between `torch.stft` and the complex multiply."""

    def __init__(self, model):
        super().__init__()
        self.m = model

    def forward(self, spec):
        m = self.m
        x = spec[:, m.freq_indices]
        x = rearrange(x, "b f t c -> b t (f c)")
        x = m.band_split(x)
        for time_transformer, freq_transformer in m.layers:
            x = rearrange(x, "b t f d -> b f t d")
            x, ps = pack([x], "* t d")
            x = time_transformer(x)
            (x,) = unpack(x, ps, "* t d")
            x = rearrange(x, "b f t d -> b t f d")
            x, ps = pack([x], "* f d")
            x = freq_transformer(x)
            (x,) = unpack(x, ps, "* f d")
        mask = m.mask_estimators[0](x)  # one stem (vocals)
        mask = rearrange(mask, "b t (f c) -> b f t c", c=2)
        batch, _, frames, _ = spec.shape
        index = m.freq_indices.view(1, -1, 1, 1).expand(batch, -1, frames, 2)
        summed = torch.zeros_like(spec).scatter_add(1, index, mask)
        denom = m.num_bands_per_freq.repeat_interleave(m.audio_channels).view(1, -1, 1, 1)
        return summed / denom.clamp(min=1e-8)


def matmul_attention(self, q, k, v):
    """`Attend.forward` without einsum: same maths, but ONNX Runtime's CPU Einsum is ~10x
    slower than MatMul for these shapes."""
    attn = (q @ k.transpose(-1, -2) * q.shape[-1] ** -0.5).softmax(dim=-1)
    return attn @ v


def full_rotary(freqs, t, start_index=0, scale=1.0, seq_dim=-2, freqs_seq_dim=None, inverse=False):
    """`apply_rotary_emb` for this model's case (rotation over the whole head, nothing left
    unrotated). The library slices off empty left/right parts, and those zero-width tensors
    split the graph into ~100 CoreML partitions."""
    import rotary_embedding_torch.rotary_embedding_torch as rotary

    assert start_index == 0 and not inverse and freqs.shape[-1] == t.shape[-1]
    freqs = freqs[-t.shape[seq_dim]:]
    return t * freqs.cos() * scale + rotary.rotate_half(t) * freqs.sin() * scale


def load(repo: Path, ckpt: Path):
    sys.path.insert(0, str(repo))
    from models.mel_band_roformer import MelBandRoformer  # noqa: E402
    from models.mel_band_roformer.attend import Attend  # noqa: E402

    import rotary_embedding_torch.rotary_embedding_torch as rotary

    Attend.forward = matmul_attention
    rotary.apply_rotary_emb = full_rotary

    config = yaml.load((repo / "configs/config_vocals_mel_band_roformer.yaml").read_text(), Loader=yaml.FullLoader)
    model_cfg = dict(config["model"])
    model_cfg["flash_attn"] = False  # plain attention: identical maths, exportable
    model = MelBandRoformer(**model_cfg)
    model.load_state_dict(torch.load(ckpt, map_location="cpu"))
    return model.eval(), config


def host_separate(model, net, audio):
    """What the Rust host does, in torch: STFT → mask → iSTFT (one chunk)."""
    window = torch.hann_window(2048)
    spec = torch.view_as_real(torch.stft(audio, 2048, 441, 2048, window, center=True, return_complex=True))
    spec = rearrange(spec, "s f t c -> 1 (f s) t c")
    mask = net(spec)
    out = torch.view_as_complex(spec.contiguous()) * torch.view_as_complex(mask.contiguous())
    out = rearrange(out, "1 (f s) t -> s f t", s=2)
    return torch.istft(out, 2048, 441, 2048, window, center=True, length=audio.shape[-1])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", type=Path, required=True)
    ap.add_argument("--ckpt", type=Path, required=True)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--step", type=int, help="chunk step in samples (default: overlap by one fade; the reference uses chunk/2)")
    args = ap.parse_args()

    model, config = load(args.repo, args.ckpt)
    net = MaskNet(model).eval()
    chunk = int(config["inference"]["chunk_size"])
    frames = chunk // 441 + 1
    bins = 2048 // 2 + 1

    with torch.no_grad():
        audio = torch.randn(2, chunk) * 0.1
        reference = model(audio[None])[0]
        ours = host_separate(model, net, audio)
        err = (reference - ours).abs().max().item()
        print(f"wrapper vs model max abs diff: {err:.2e}")
        assert err < 1e-4, "MaskNet does not reproduce the model"

        dummy = torch.zeros(1, 2 * bins, frames, 2)
        args.out.parent.mkdir(parents=True, exist_ok=True)
        torch.onnx.export(net, (dummy,), str(args.out), input_names=["spec"], output_names=["mask"],
                          opset_version=17, dynamo=False)
    sidecar = {
        "name": "melband_roformer_vocals",
        "family": "melband_roformer",
        "sources": ["vocals"],
        "samplerate": 44100,
        "audio_channels": 2,
        "chunk_samples": chunk,
        "step_samples": args.step or chunk - chunk // 10,
        "nfft": 2048,
        "hop": 441,
        "freq_bins": bins,
        "frames": frames,
    }
    args.out.with_suffix(".json").write_text(json.dumps(sidecar, indent=2) + "\n")
    print("wrote", args.out, "and", args.out.with_suffix(".json"))


if __name__ == "__main__":
    main()
