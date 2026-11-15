# Grain ML sidecar

Optional, opt-in AI generation for Grain's Produce view (AGENTS.md §8): ACE-Step 1.5 behind a
small FastAPI app. The desktop app installs it from the Models panel ("Install AI generation"),
starts it for one generation on `127.0.0.1:<free port>` with a random token
(`GRAIN_SIDECAR_TOKEN`), and stops it when the generation ends. The HTTP contract is in
`grain_sidecar/server.py`.

- Install (what the app runs, into its app-data folder): `uv sync --frozen --extra acestep`,
  then `python -I -m grain_sidecar download --checkpoints <dir>`.
- Footprint: Python env with PyTorch ~2 GB; weights ~6.3 GB (`acestep-v15-base` 4.8 GB, VAE
  0.3 GB, Qwen3-Embedding-0.6B text encoder 1.2 GB). ACE-Step itself and its weights are MIT.
- Tests (stub backend, no model): `uv run --frozen python -I -m pytest -q tests`.
