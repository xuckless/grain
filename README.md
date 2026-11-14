# music_analysis

A singing and music workstation: stem separation, a multitrack timeline and performance scoring.
It ships as a desktop app (Tauri) and a web app (WASM), both built on one Rust engine.
See [AGENTS.md](AGENTS.md) for the product spec and engineering rules.

## Layout

| Path | What |
|---|---|
| `crates/engine-core` | DSP and analysis: YIN, STFT, tempo, key, melody notes, scoring, mixer/transport, project model. Also builds for wasm32. |
| `crates/engine-ml` | HTDemucs host processing (STFT outside the graph, chunking, overlap-add) |
| `crates/engine-native` | Desktop host: decode, resample, cpal playback, ONNX Runtime, stem cache |
| `crates/engine-wasm` | Web bindings (main thread + AudioWorklet) |
| `crates/ingest` | YouTube audio through a local `yt-dlp` + `ffmpeg` (desktop only) |
| `crates/cli` | `music` CLI: `compare` (port of `prototype/main.py`), `import`, `score` |
| `packages/engine-api` | TypeScript types generated from Rust, plus the host contract |
| `packages/ui` | Shared Svelte 5 UI in the Grain design system: `theme.ts` (colours), `motion.ts` (all timing), `grain.ts` (every dot/particle drawing) |
| `apps/web`, `apps/desktop` | The two hosts |

## Setup

You need Rust (with the `wasm32-unknown-unknown` target), `wasm-pack`, Node 22 and pnpm. YouTube import also needs `yt-dlp` and `ffmpeg` on `PATH`.

```bash
python3 scripts/fetch_models.py   # separation weights → models/weights/ (optional; without them songs stay one mix track)
pnpm install
```

## Run

```bash
pnpm dev:web          # http://localhost:5173 (serves models/weights at /models)
pnpm dev:desktop      # Tauri window
cargo run --release -p music-cli -- import song.mp3 --model models/weights/htdemucs_6s.onnx
cargo run --release -p music-cli -- score song.mp3 my-take.m4a --model models/weights/htdemucs_6s.onnx
cargo run --release -p music-cli -- compare mine.wav artist.wav
```

## Checks

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p engine-core --target wasm32-unknown-unknown   # engine-core DSP tests under Node
pnpm types && pnpm wasm && pnpm check && pnpm build:web
python3 scripts/fetch_models.py --check
```

The prototype is in `prototype/`. License: GPL-3.0.

## Fonts

Geist and Geist Mono are bundled from `@fontsource-variable/*` (SIL OFL 1.1). `packages/ui/src/assets/GeistPixelCircle.woff2`
(wordmark, hero and score reveal only) comes from the Grain design canvas. It's believed to be Vercel's Geist Pixel
(Geist family, OFL 1.1), but that licence is **unverified**. Confirm it before distributing a build.
