# Grain guide

A singing and music workstation: stem separation, a multitrack timeline, performance scoring,
live karaoke, auto-tune, a pitch editor and AI production. It ships as a desktop app (Tauri) and
a web app (WASM), both built on one Rust engine. See [AGENTS.md](../AGENTS.md) for the product spec
and engineering rules.

## What it does

| Feature | Where | Desktop | Web |
|---|---|---|---|
| Import a file or a YouTube link, split it into stems (HTDemucs; optional Mel-Band RoFormer "Best vocals"), find tempo, key and the vocal melody | Home | ✓ | files only; Fast separation |
| Multitrack mix: compact rows with instrument icons, mute/solo, gain and pan knobs, shared zoom, scrubbing; MIDI lanes from basic-pitch or the drum classifier, `.mid` export | Mix | ✓ | ✓ |
| Score a take against the melody (pitch, timing, stability), A/B a note; import takes | Score | ✓ | ✓ |
| Live karaoke: record takes, LRCLIB lyrics, scrolling note lane, live pitch trail and streaks, seek, 2 s pre-roll, loop A/B | Karaoke | ✓ | ✓ |
| Auto-tune: live monitor (dry or tuned), applied to the take as a chain step | Auto-tune | ✓ | ✓ |
| Melodyne-style pitch editor: drag notes, stretch, split, vibrato, drift, formant; ghost notes of the melody, snap to original, A/B | Pitch | ✓ | ✓ |
| Produce: DeepFilterNet clean-up, vocal chain (live and offline), harmonies, melody MIDI, ACE-Step backing | Produce | ✓ | chain, harmony, MIDI; generation only with a remote backend |
| Model manager: download, verify and delete every model; opt-in licences; AI generation install | Models | ✓ | ✓ (no generation install) |

Everything is non-destructive. Each take keeps its raw recording and one processing chain, Pitch
edits → Auto-tune → Clean up → Vocal chain, each step switchable on and off; ★ marks the master
take that views and export start from. ⌘Z / ⇧⌘Z undo mix, take, pitch and chain changes.
**Export…** in the top bar writes a mixdown (WAV 24-bit; MP3 and M4A on desktop through the bundled
ffmpeg) or the `.grain` project file. Harmonies and generations are new tracks. ⚙ Settings holds
the playback mode (voice only, + instrumental, + full mix; never the guide vocal), latency
calibration and monitoring; every view has its tips under (i) and rare actions under ⋯.

## Layout

| Path | What |
|---|---|
| `crates/engine-core` | DSP and analysis: YIN, STFT, resampling, tempo, key, melody notes, DTW alignment, scoring, mixer/transport, project model. Also builds for wasm32. |
| `crates/engine-ml` | Model host processing (HTDemucs, RoFormer, RMVPE, basic-pitch) and the model registry compiled from `models/manifest.toml` |
| `crates/engine-native` | Desktop host: decode, resample, cpal playback, ONNX Runtime, stem cache |
| `crates/engine-wasm` | Web bindings (main thread + AudioWorklet) |
| `crates/ingest` | YouTube audio through `yt-dlp` + `ffmpeg`: bundled sidecars, else `PATH` (desktop only) |
| `crates/cli` | `music` CLI: `compare` (port of `prototype/main.py`), `import`, `score` |
| `packages/engine-api` | TypeScript types generated from Rust, plus the host contract |
| `packages/ui` | Shared Svelte 5 UI in the Grain design system: `theme.ts` (colours), `motion.ts` (all timing), `grain.ts` (every dot/particle drawing) |
| `apps/web`, `apps/desktop` | The two hosts |
| `services/ml-sidecar` | Opt-in Python sidecar (FastAPI, managed by `uv`) for ACE-Step 1.5 generation |
| `third_party/deep-filter-rt` | Vendored DeepFilterNet3 runtime (MIT OR Apache-2.0; see its `NOTICE.md`) |

## Setup

You need Rust (with the `wasm32-unknown-unknown` target), `wasm-pack`, Node 22 and pnpm. YouTube import in dev builds uses `yt-dlp` and `ffmpeg` from `PATH`; release builds can bundle them (below).

```bash
python3 scripts/fetch_models.py   # weights → models/weights/ (optional; without them songs stay one mix track, pitch uses YIN)
pnpm install
```

In the app, **Models** (top right, on Home and in a song) lists every model in
`models/manifest.toml` with its size, licence and state, and downloads, verifies (SHA-256) and
deletes them. The desktop app keeps them in its data folder
(`~/Library/Application Support/dev.workstation.app/models` on macOS; debug builds also read
`models/weights/`), the web app in the browser's private storage (OPFS; `pnpm dev:web` also serves
`models/weights/`). Restricted (non-commercial, `opt-in`) models ask you to accept their licence
first. Downloads only come from Hugging Face and GitHub.

`fetch_models.py` downloads the same models for development (HTDemucs, RMVPE, basic-pitch). The
Best-vocals model (Mel-Band RoFormer) is not hosted as ONNX: if it isn't built yet, the script
downloads its checkpoint to `models/build/` and prints the export command
(`scripts/export_melband_roformer.py`, needs PyTorch). MuScriptor (CC-BY-NC-4.0) is listed but
gated on Hugging Face and PyTorch-only, so it can't be downloaded yet.

## Run

```bash
pnpm dev:web          # http://localhost:5173 (serves models/weights at /models)
pnpm dev:desktop      # Tauri window
cargo run --release -p music-cli -- import song.mp3 --models models/weights [--quality best-vocals]
cargo run --release -p music-cli -- score song.mp3 my-take.m4a --models models/weights
cargo run --release -p music-cli -- pitch vocals.wav --models models/weights   # YIN vs RMVPE
cargo run --release -p music-cli -- compare mine.wav artist.wav
```

## Desktop release with bundled yt-dlp and ffmpeg

```bash
python3 scripts/fetch_sidecars.py      # pinned GitHub releases → apps/desktop/src-tauri/binaries/, sha256-checked
pnpm --filter desktop tauri build --bundles app --config src-tauri/tauri.bundle.conf.json
```

`sidecars.toml` (next to `tauri.conf.json`) pins the URLs and checksums per target; pass
`--target <rust-triple>` to fetch for another platform. A plain `pnpm build:desktop` needs no
sidecars and finds the tools on `PATH`. In the app, Home shows which yt-dlp is in use and an
**Update extractor** button (yt-dlp's own `-U`, applied to a copy in the app's data folder).

## Recording a take

Open a song, go to **Karaoke** and press **● Sing** (bottom centre). The song starts 2 s before the
playhead, you hear a one-bar count-in, and the microphone records until **Pause** or **Finish**.
Each run is lined up with the song, scored, and kept with the project; **Score** shows the score of
the take picked in its toolbar (★ the master by default) and imports takes from ⋯. Click a note in
the graph to hear the original, then your take.

- **Your takes:** every take is saved with the song the moment it exists (recorded or imported)
  and listed under **Your takes** in Score and Karaoke. Play one as **Voice only**, **Voice +
  instrumental** or **Voice + full mix** (also in ⚙ Settings), rename (undoable) or delete it.
  Playing a take stops the song, and pressing Play stops the take.
- **Backing** (Karaoke toolbar, what you hear while singing): *Instrumental* (the song without its vocal stem), *Instrumental + guide vocal* (the
  original vocal quietly) or *Full mix*. The default is Instrumental, except for songs whose
  non-vocal stems are more than 30 dB below the mix (a cappella), which default to the guide vocal.
- **Headphones** keep the backing out of your take.
- **Latency** (⚙ Settings): press **Calibrate** once per microphone and output. Unplug your headphones, or hold
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

- Move to where you want to start (drag the lane, click the ruler under it, or click a lyric line)
  and press **● Sing**: the song starts 2 s early, a one-bar count-in, then it plays with the
  backing you chose in the toolbar.
- **Loop:** press it once at A and again at B. Every pass is saved as its own take and the next
  pass counts in again from A.
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

## Auto-tune

Open a song and go to **Auto-tune**.

- **Key** defaults to the song's detected key. You can also pick another key the analysis found, Chromatic, or **Custom notes** (tap the keys of the one-octave keyboard). **Pull to** chooses the nearest scale note, or **Follow the melody**, which pulls to the song's own melody in your octave.
- **Retune speed:** 0 is the hard, robotic snap; 50–400 ms sounds natural. **Humanize** lets held notes keep their vibrato. **Formant** shifts the voice's character without changing pitch. **Mix** blends the dry voice back in.
- **Hear yourself live** monitors the microphone over the song, dry or **Tuned**. Use headphones: speakers feed back. The hint shows how far behind you the monitor is (about 32 ms for the tuner, plus your devices' latency).
- **Apply to take** (bottom centre) puts auto-tune on the picked take with the high-quality offline shifter; the raw recording is kept, **On** bypasses it, ⋯ removes it, ⌘Z undoes it. Before you apply, the graph shows your pitch (grey), the notes it is pulled to, and the corrected pitch (pink).

## Pitch editor

Open a song and go to **Pitch**. The toolbar's take picker chooses the take (★ the master by default); every tool is in the toolbar.

- Each sung note is a blob of dots (thicker where it's louder) on a piano roll, with the scale's
  notes as dotted rows, your pitch in grey and the edited pitch over it.
- **Drag a note up or down** to change its pitch: it snaps to **Scale notes** or **Semitones**
  (Snap menu); hold **⌥ Alt** for free cents. Drag it **sideways** to move it in time, or by an
  **edge** to stretch it (0.5–2×). Click selects (⇧/⌘ adds), ⌘A selects all, Esc clears.
- **Split notes** (Tool menu) splits where you click; **Merge with next** joins a selected note
  with the following one.
- The sliders work on the selection, or on every note when none is selected: **Pitch centre**
  (how far each note is pulled to its scale note), **Pitch drift** (flattens slow wander),
  **Vibrato** (0–200%), **Glide in** (how fast the pitch moves in from the previous note),
  **Formant**. **Correct pitch** does centre 100% + drift 50% in one go. **Delete** resets the
  selected notes; ⌘Z / ⇧⌘Z undo and redo.
- The original melody shows as ghost notes; each note shows its offset in cents. **Snap to
  original** moves notes onto the melody; **A/B original** plays the original vocal, then the take.
- **Apply edits to take** (bottom centre) puts the edits on the take's chain, re-rendering only the
  changed notes; **On** bypasses them.
- From **Auto-tune**, **Edit curve…** opens the take in the editor with every note set where
  auto-tune would pull it, so you can adjust the correction note by note.

## MIDI

In **Mix**, **MIDI** on a track transcribes it: pitched tracks with basic-pitch (download it in
**Models** first), the drum stem with a built-in kick/snare/hat classifier. The notes appear as a
lane under the track; **Export .mid** saves a Standard MIDI File at the song's tempo, and **Redo**
transcribes again.

## Produce

Open a song and go to **Produce**: stacked cards over the picked take (★ the master by default).
Clean up and the vocal chain are steps of the take's chain (on/off, Remove, ⌘Z); harmonies and
generations are new tracks listed under **Made here** with their own play toggle, level and pan.

- **Clean up** (desktop) removes room noise, hum and hiss with DeepFilterNet 3 (download
  `deepfilternet3`, 8 MB, in **Models** first). It runs at 48 kHz on the CPU, about 50× faster than
  real time. Noise is pulled down by at most 30 dB so breaths stay.
- **Vocal chain:** high-pass → de-esser → compressor → EQ (low, presence, air) → saturation →
  reverb and delay. **Preset** sets every control from the take's own level: *Auto* also measures
  its brightness and sibilance and corrects them (a dark take gets more presence and air, a sibilant
  one more de-essing); *Natural*, *Pop*, *R&B*, *Rock*, *Hip-hop* and *Ballad* keep their own tone.
  Tweak any slider, switch on **Hear it live** to sing through it over the song (headphones), and
  **Apply to take** to put it on the take. The meters show the take's loudness (LUFS), peak,
  brightness and sibilance.
- **Harmony:** **Third above/below** and **Sixth above/below** sing a diatonic harmony in the song's
  key from the singer's own voice (pitch-shifted with formants kept), as a new track that plays with
  the song, quieter and panned. **Melody MIDI** exports the melody (or the last harmony) as `.mid`.
- **Generate** (desktop, opt-in): ACE-Step 1.5 makes a **Backing for the vocal**, **One instrument**
  (drums, bass, guitar, keys, strings, …) or **Repaints** a section of the mix, conditioned on the
  song's tempo, key and the source vocal, plus your style prompt and a seed. Install it once from
  **Models → Install AI generation**: it needs [`uv`](https://docs.astral.sh/uv/), then downloads
  about 2 GB of Python packages (PyTorch) and 6.3 GB of weights (the 2B base model, its VAE and text
  encoder) into the app's data folder. Each generation starts a local helper on 127.0.0.1 with a
  one-time key, loads the model (MPS on Apple silicon, else CPU), and quits when done, so nothing
  stays in memory. Expect minutes per song on Apple silicon and much longer on CPU. The web build
  offers generation only when built with `VITE_GRAIN_GENERATION_URL` and
  `VITE_GRAIN_GENERATION_TOKEN` pointing at a sidecar you run elsewhere.

## Checks

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p engine-core --target wasm32-unknown-unknown   # engine-core DSP tests under Node
pnpm types && pnpm wasm && pnpm check && pnpm build:web
pnpm --filter @workstation/ui test                   # recording, calibration, listening, pitch editor, model manager
python3 scripts/fetch_models.py --check
(cd services/ml-sidecar && uv run --frozen python -I -m pytest -q tests)   # sidecar HTTP contract (stub backend)
cargo test --release -p engine-native -- --ignored   # RMVPE on synthetic sines (needs weights); a real model download
```

CI (`.github/workflows/ci.yml`) runs the same checks on macOS, Windows and Linux.

### UI frame times

`scripts/perf/run.mjs` drives the web build in headless Chrome (frame-rate limit off, GPU raster,
device pixel ratio 2) and prints frame-time p50/p95/p99/max, long tasks and main-thread → audio
messages per second for: Home idle, Mix idle, a gain-knob turn, playback, a drag while playing,
scrubbing, tab switching, each view idle, Pitch playback and a pitch-editor drag (AGENTS.md §10.1).

```bash
pnpm build:web
pnpm --filter web preview --port 4400 &
node scripts/perf/run.mjs http://localhost:4400 samples/your-song.mp3 out.json   # DPR=1, SOFTWARE=1, CHROME=… to vary
kill %1
```

With the frame-rate limit off most frames have no input to handle, so p99 is the number that
shows a drag's cost.

The prototype is in `prototype/`. License: GPL-3.0.

## Fonts

Geist and Geist Mono are bundled from `@fontsource-variable/*` (SIL OFL 1.1). `packages/ui/src/assets/GeistPixelCircle.woff2`
(wordmark, hero and score reveal only) comes from the Grain design canvas. It's believed to be Vercel's Geist Pixel
(Geist family, OFL 1.1), but that licence is **unverified**. Confirm it before distributing a build.
