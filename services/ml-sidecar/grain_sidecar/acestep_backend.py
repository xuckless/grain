"""ACE-Step 1.5 (github.com/ace-step/ACE-Step-1.5, MIT; weights on Hugging Face, MIT) as the
generation backend. Loaded on the first request of a run and released when the process exits:
the app starts the sidecar for one generation and stops it afterwards (low memory).

Every task uses the 2B *base* DiT (`acestep-v15-base`): `lego` and `complete` are only
supported by base models, and XL-base needs 20 GB+ of (unified) memory. The 5 Hz LM is not
used (`thinking=False`), so its weights are not downloaded.

Not exercised in development (no weights were downloaded); the HTTP contract and lifecycle are
tested with the stub backend.
"""

import os
import tempfile
from pathlib import Path

DIT = "acestep-v15-base"
MAIN_REPO = "ACE-Step/Ace-Step1.5"
# From the main repo only the VAE and the text encoder (not the turbo DiT or the LM).
MAIN_PARTS = ["vae/*", "Qwen3-Embedding-0.6B/*"]

TRACK_INSTRUCTION = "Generate the {track} track based on the audio context:"
COMPLETE_INSTRUCTION = "Complete the input track with drums, bass, guitar, keyboard:"


def download_weights(checkpoints: str):
    """~6.3 GB: the base DiT (4.8 GB), the VAE (0.3 GB) and the text encoder (1.2 GB)."""
    from huggingface_hub import snapshot_download

    root = Path(checkpoints)
    snapshot_download(MAIN_REPO, local_dir=root, allow_patterns=MAIN_PARTS)
    snapshot_download(f"ACE-Step/{DIT}", local_dir=root / DIT)


def pick_device():
    import torch

    if torch.cuda.is_available():
        return "cuda"
    if torch.backends.mps.is_available():
        return "mps"
    return "cpu"


class AceStepBackend:
    name = "acestep"

    def __init__(self, checkpoints):
        if not checkpoints:
            raise SystemExit("--checkpoints is required for the acestep backend")
        self.checkpoints = Path(checkpoints)
        os.environ["ACESTEP_CHECKPOINTS_DIR"] = str(self.checkpoints)
        self.handlers = None

    def device(self):
        return pick_device()

    def load(self):
        if self.handlers is None:
            from acestep.handler import AceStepHandler
            from acestep.llm_inference import LLMHandler

            dit = AceStepHandler()
            dit.initialize_service(
                project_root=str(self.checkpoints.parent),
                config_path=DIT,
                device=pick_device(),
            )
            self.handlers = (dit, LLMHandler())
        return self.handlers

    def generate(self, *, task, prompt, seed, bpm, key, duration, track, start, end, audio):
        from acestep.inference import GenerationConfig, GenerationParams, generate_music

        dit, llm = self.load()
        with tempfile.TemporaryDirectory() as tmp:
            src = Path(tmp) / "source.wav"
            src.write_bytes(audio)
            instruction = {
                "complete": COMPLETE_INSTRUCTION,
                "lego": TRACK_INSTRUCTION.format(track=track),
                "repaint": None,
            }[task]
            params = GenerationParams(
                task_type=task,
                src_audio=str(src),
                caption=prompt,
                bpm=int(round(bpm)),
                keyscale=key,
                duration=duration,
                seed=seed,
                inference_steps=50,
                thinking=False,
                repainting_start=start if task == "repaint" else 0.0,
                repainting_end=end if task == "repaint" else -1,
                **({"instruction": instruction} if instruction else {}),
            )
            config = GenerationConfig(
                batch_size=1, use_random_seed=False, seeds=[seed], audio_format="wav"
            )
            result = generate_music(dit, llm, params, config, save_dir=tmp)
            if not result.success:
                raise RuntimeError(result.error)
            return Path(result.audios[0]["path"]).read_bytes()
