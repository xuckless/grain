#!/usr/bin/env python3
"""PyTorch reference for the vocal cascade (AGENTS.md §4.2), to check the Rust host against.

Runs the upstream inference code unmodified: KimberleyJensen's `demix_track` for the vocals,
then Meta's `demucs.apply.apply_model` (htdemucs_6s) on mix - vocals, with its vocal output
folded into `other`. Optionally compares against a stem directory written by our engine.

Usage:
  python -I scripts/reference_cascade.py --repo <Mel-Band-Roformer-Vocal-Model> \
      --ckpt MelBandRoformer.ckpt --song song.wav [--ours <stem dir>] [--out <dir>]
"""

import argparse
import gc
import sys
from pathlib import Path

import numpy as np
import soundfile as sf
import torch
import yaml


def db(x):
    return 10 * np.log10(np.mean(x ** 2) + 1e-20)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", type=Path, required=True)
    ap.add_argument("--ckpt", type=Path, required=True)
    ap.add_argument("--song", type=Path, required=True)
    ap.add_argument("--ours", type=Path)
    ap.add_argument("--out", type=Path)
    args = ap.parse_args()

    sys.path.insert(0, str(args.repo))
    from ml_collections import ConfigDict  # noqa: E402
    from models.mel_band_roformer import MelBandRoformer  # noqa: E402
    from utils import demix_track  # noqa: E402
    from demucs.apply import apply_model  # noqa: E402
    from demucs.pretrained import get_model  # noqa: E402

    config = ConfigDict(yaml.load((args.repo / "configs/config_vocals_mel_band_roformer.yaml").read_text(), Loader=yaml.FullLoader))
    roformer = MelBandRoformer(**dict(config.model))
    roformer.load_state_dict(torch.load(args.ckpt, map_location="cpu"))
    roformer.eval()

    mix, sr = sf.read(args.song, dtype="float32", always_2d=True)
    mix = torch.tensor(mix.T)
    with torch.inference_mode():
        vocals = torch.tensor(demix_track(config, roformer, mix, "cpu")[0]["vocals"])
    del roformer
    gc.collect()

    demucs = get_model("htdemucs_6s").eval()
    inst = mix - vocals
    ref = inst.mean(0)
    norm = (inst - ref.mean()) / ref.std()
    with torch.inference_mode():
        out = apply_model(demucs, norm[None], split=True, overlap=0.25, progress=False)[0]
    out = out * ref.std() + ref.mean()
    stems = {name: s.numpy() for name, s in zip(demucs.sources, out)}
    stems["other"] = stems["other"] + stems.pop("vocals")
    stems["vocals"] = vocals.numpy()

    print(f"mix {db(mix.numpy()):6.1f} dBFS")
    for name, s in stems.items():
        line = f"{name:7s} ref {db(s):6.1f} dBFS"
        if args.ours:
            o, _ = sf.read(args.ours / f"{name}.wav", dtype="float32", always_2d=True)
            o = o.T[:, : s.shape[1]]
            sdr = 10 * np.log10(np.sum(s ** 2) / (np.sum((s - o) ** 2) + 1e-20))
            line += f"  ours {db(o):6.1f} dBFS  agreement {sdr:5.1f} dB"
        print(line)
        if args.out:
            args.out.mkdir(parents=True, exist_ok=True)
            sf.write(args.out / f"{name}.wav", s.T, sr)


if __name__ == "__main__":
    main()
