# music_analysis

A singing and music workstation: stem separation, a multitrack timeline and performance scoring.
It ships as a desktop app (Tauri) and a web app (WASM), both built on one Rust engine.
See [AGENTS.md](AGENTS.md) for the product spec and engineering rules.

## Layout

| Path | What |
|---|---|
| `crates/engine-core` | DSP and analysis: YIN, STFT, resampling, tempo, key, melody notes, DTW alignment, scoring, mixer/transport, project model. Also builds for wasm32. |
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
python3 scripts/fetch_models.py   # weights → models/weights/ (optional; without them songs stay one mix track, pitch uses YIN)
pnpm install
```

`fetch_models.py` downloads HTDemucs and RMVPE. The Best-vocals model (Mel-Band RoFormer) is
not hosted as ONNX: the script downloads its checkpoint to `models/build/` and prints the export
command (`scripts/export_melband_roformer.py`, needs PyTorch). The desktop app reads weights from
its data folder (`~/Library/Application Support/dev.workstation.app/models` on macOS); debug
builds also read `models/weights/`.

## Run

```bash
pnpm dev:web          # http://localhost:5173 (serves models/weights at /models)
pnpm dev:desktop      # Tauri window
cargo run --release -p music-cli -- import song.mp3 --models models/weights [--quality best-vocals]
cargo run --release -p music-cli -- score song.mp3 my-take.m4a --models models/weights
cargo run --release -p music-cli -- pitch vocals.wav --models models/weights   # YIN vs RMVPE
cargo run --release -p music-cli -- compare mine.wav artist.wav
```

## Recording a take

Open a song, go to **Score** and press **Record a take**. You hear a one-bar count-in at the song's
tempo, then the song plays from the playhead while the microphone records; press **Stop**, or let
the song end. The take is lined up with the song, scored, and kept with the project (pick earlier
takes in the **Take** menu; click a note in the graph to hear the original, then your take).

- **Backing:** *Instrumental* (the song without its vocal stem), *Instrumental + guide vocal* (the
  original vocal quietly) or *Full mix*. The default is Instrumental, except for songs whose
  non-vocal stems are more than 30 dB below the mix (a cappella), which default to the guide vocal.
- **Headphones** keep the backing out of your take.
- **Latency:** press **Calibrate** once per microphone and output. Unplug your headphones, or hold
  them against the microphone, so it can hear a short chirp. The measured round-trip delay is
  stored per device pair and removed from every take; nudge it with − / + if takes still sound
  early or late. Uncalibrated, desktop assumes 0 ms beyond what the drivers report, and the web uses
  the browser's output-latency estimate.
- **Web:** projects are stored in the browser (OPFS) and listed under Recent; **Export** downloads
  a `.grain` file (stems and takes as 16-bit PCM plus the project), and **Open project file…**
  reopens one. Recording needs the cross-origin isolation headers the dev and preview servers send.

## Karaoke

Open a song and go to **Karaoke** (it needs a vocal melody to sing against). The first time a song
opens there, its lyrics are looked up on [LRCLIB](https://lrclib.net): the artist and title are
guessed from the video title or file name (`Artist - Title.mp3`), and you can correct them and
**Search** again. Only the artist, title and song length are sent. Found lyrics are kept with the
project, and **Save** keeps them (and any edits).

- Seek to where you want to start and press **Sing**: a one-bar count-in, then the song plays with
  the backing you chose (the same choice and latency calibration as Score).
- The lane scrolls the song's notes past the playhead. Your voice draws a trail: white is in tune,
  amber is close, violet is off. Notes fill as you pass them, hits burst and build your **Streak**,
  and **Score** is the share of the melody you've sung in tune. Octave folding lets you sing an
  octave up or down.
- Two lyric lines show at a time, and the current one wipes syllable by syllable (Japanese and
  Chinese by character), timed to the song's notes.
- **Pause** saves what you've sung so far as a take; **Resume** counts you back in. **Finish** (or the
  song's end) saves the last run. Each take is scored in full in **Score**.
- Lyrics early or late? Use **Lyrics offset** − / + (100 ms steps), or **Edit lines** to fix a line's
  text or start time.

## Checks

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p engine-core --target wasm32-unknown-unknown   # engine-core DSP tests under Node
pnpm types && pnpm wasm && pnpm check && pnpm build:web
pnpm --filter @workstation/ui test                   # recording, calibration and A/B state machines
python3 scripts/fetch_models.py --check
cargo test --release -p engine-native -- --ignored   # RMVPE on synthetic sines (needs weights)
```

CI (`.github/workflows/ci.yml`) runs the same checks on macOS, Windows and Linux.

The prototype is in `prototype/`. License: GPL-3.0.

## Fonts

Geist and Geist Mono are bundled from `@fontsource-variable/*` (SIL OFL 1.1). `packages/ui/src/assets/GeistPixelCircle.woff2`
(wordmark, hero and score reveal only) comes from the Grain design canvas. It's believed to be Vercel's Geist Pixel
(Geist family, OFL 1.1), but that licence is **unverified**. Confirm it before distributing a build.
