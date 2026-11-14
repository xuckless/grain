# AGENTS.md

Guidance for humans and coding agents working in this repository.

> **Status:** M0 and most of M1 are built (see README.md for running it). Remaining M1 gaps are listed under §11. The original PyQt5 prototype is kept in `prototype/` as a reference. Project-level decisions are recorded in [§0](#0-decisions-locked). Anything still marked **(proposed)** is a default that applies until the owner decides otherwise.

---

## Ground rules (mandatory, they override everything else in this file)

1. **NO IMPLICIT STATE MACHINES.** Any stateful behavior (transport, recording, import jobs, model downloads, UI modes, etc.) must be modeled as an explicit state machine: one enum of states, explicit transitions, and illegal transitions rejected. Do not derive state from combinations of booleans, nullable fields or flags.
2. **ANY STATE THAT IS NOT EXPLICITLY DEFINED IS NOT PART OF THE CODEBASE.** If a state is not declared in a type, it does not exist. Code must not depend on undeclared or emergent states.
3. **CODE DUPLICATION IS STRICTLY FORBIDDEN.** Logic lives in exactly one place. Shared DSP and analysis code lives in `engine-core`, shared UI in `packages/ui`, and shared types are generated once (Rust → TS). Before adding code, search for an existing implementation and reuse or extend it.
4. **THE IMPLEMENTATION SHOULD BE THE SIMPLEST POSSIBLE IMPLEMENTATION OF A PARTICULAR TASK DEFINED.** No speculative abstraction, generality or configurability beyond what the current task requires. Upgrades listed elsewhere in this file (e.g. a better model) are added when that task is scheduled, not before.

---

## 0. Decisions (locked)

| Topic | Decision | Consequences |
|---|---|---|
| License | **Open source, GPL-3.0** (`LICENSE`) | GPL code is allowed (e.g. Rubber Band). Every dependency must be GPL-compatible: MIT, Apache-2.0, BSD and LGPL are fine; proprietary SDKs are not. |
| Non-commercial weights | **Opt-in download only** | CC-BY-NC weights (MuScriptor, MMS aligner, some RoFormer checkpoints) are never bundled or downloaded by default. The model manager shows their license and requires the user to accept it. Every feature must work with the default permissive or GPL models. |
| Compute | **Local first** | Every feature runs on the user's machine. A remote or GPU-sidecar `Inference` backend is an optional add-on, never required. On the web, heavy models run on WebGPU, and features too heavy for the machine are disabled with an explanation. |
| UI | **Svelte 5 + TypeScript** in **Tauri 2** (desktop) and Vite (web) | One shared UI package. Real-time views use Canvas or WebGL. |
| Engine | **Rust** | See §2. |
| YouTube import | **Desktop only**, using a local `yt-dlp` sidecar | The web version supports local file import only. No hosted ingest service. |
| Platforms (v1) | **macOS (Apple Silicon), Windows, Linux, Web** | ML execution providers: CoreML on macOS, DirectML (or CUDA if present) on Windows, CUDA/ROCm or CPU on Linux, WebGPU with a WASM fallback on the web. |
| Accounts / storage | **Local only**, with no backend | Desktop stores projects as files. The web version uses OPFS, plus export and import as a single project archive. |
| First milestone | **F1 (stems) + timeline + F2 (scoring)** | See §11. |

---

## 1. Product

The product is a singing and music workstation. It is a desktop app with a web version, built on one shared engine.

| # | Feature | Summary |
|---|---------|---------|
| F1 | **Import & stem split** | Paste a YouTube link. The app gets the audio, separates it into stems (vocals, drums, bass, guitar, piano, other), optionally transcribes the stems to MIDI, and lays them out on a multitrack timeline like GarageBand. Each track has mute, solo, volume and pan. |
| F2 | **Performance scoring** | The user records or imports their own singing of the song. The app aligns it to the original vocal and shows scores (pitch, timing, stability) plus a graph of where they were sharp, flat, early or late. |
| F3 | **Live karaoke** | Lyrics are fetched on demand and synced to the music. The user sings along live. A scrolling note lane shows the reference vocal notes and the user's live pitch, and highlights hits and misses as they happen. |
| F4 | **Auto-tune** | Pitch correction based on DSP, not ML. It snaps the voice to the song's key or scale, with adjustable retune speed and humanize controls. It runs live (low latency) and offline (high quality). |
| F5 | **AI production** | AI voice enhancement (denoise, de-reverb, restoration). It also generates melodies, backing tracks and missing instruments to turn a rough vocal into a fairly complete song. |

The source audio is lossy (YouTube Opus/AAC at about 128–160 kbps). That is accepted: every algorithm must cope with codec artifacts, and nothing should assume lossless input.

---

## 2. Architecture

```
┌───────────────────────────── UI (shared web frontend) ─────────────────────────────┐
│  Timeline / mixer · Karaoke lane · Score graphs · Lyrics · Autotune panel          │
│  Svelte 5 + TypeScript, Canvas2D/WebGL for timeline & pitch lanes                  │
└───────────────▲─────────────────────────────────────────────▲──────────────────────┘
                │ Tauri IPC (desktop)                         │ wasm-bindgen + postMessage (web)
┌───────────────┴──────────────┐                ┌─────────────┴───────────────────────┐
│ Desktop host (Tauri 2, Rust) │                │ Web host (browser)                  │
│  engine-native: cpal audio   │                │  AudioWorklet ← engine.wasm (DSP)   │
│  ort (ONNX Runtime) for ML   │                │  Web Worker  ← onnxruntime-web      │
│  symphonia decode            │                │               (WebGPU → WASM fallbk)│
└───────────────▲──────────────┘                └─────────────▲───────────────────────┘
                │            same Rust crate: engine-core     │
                └───────────────────┬─────────────────────────┘
                                    │ HTTP (optional, opt-in; never required)
                   ┌────────────────┴────────────────┐
                   │ Optional services / sidecars    │
                   │  (yt-dlp is a desktop sidecar)  │
                   │  ml-heavy: Python, GPU          │
                   │   (ACE-Step, RoFormer, enhance) │
                   └─────────────────────────────────┘
```

### Language: Rust

Rust is the default for the engine. C++ was the alternative. Rust wins on four counts:
- **Web is a first-class target.** `wasm32-unknown-unknown` with `wasm-bindgen` is far simpler than Emscripten. Pure-Rust crates compile to WASM without a libc.
- **Real-time safety.** Ownership rules make the audio-thread rules in §9 much easier to enforce.
- **Ecosystem:** `cpal` for audio I/O, `symphonia` for decoding, `rubato` for resampling, `realfft`/`rustfft` for FFTs, `rtrb` for lock-free ring buffers, `ort` for ONNX Runtime, and `wgpu` if we need it.
- **Cargo workspace.** One build covers native, WASM and the Tauri host.

C++ libraries are allowed only behind a `-sys` crate, and only if they build for `wasm32-unknown-unknown` or are native-only. Many C++ libraries need libc++, which is painful on wasm32. Prefer a Rust port, or our own implementation, for anything that has to run on the web.

### UI: Svelte 5 frontend in a Tauri shell

The same TypeScript UI runs in Tauri on desktop and in the browser on the web. The waveform, timeline and pitch lanes are drawn with Canvas2D or WebGL, because the DOM is too slow at 60 fps with many notes. Native-Rust UIs (egui, iced, Slint) were rejected because the web version would then need either a second UI or a weaker canvas UI.

### Where ML runs

| Model class | Desktop | Web | Notes |
|---|---|---|---|
| Pitch (RMVPE/FCPE/SwiftF0, <30 MB) | local `ort` | local onnxruntime-web | Must be local for live karaoke |
| Separation, light (HTDemucs, ~80 MB/model) | local `ort` (CPU; DirectML/CUDA EP where verified. **CoreML EP produces NaN on htdemucs_6s and is ~20× slower; do not enable until it passes the reference check**) | local WebGPU (WASM fallback, slow) | A 3–4 min song takes tens of seconds on GPU and several minutes on CPU |
| Separation, best (BS/Mel-RoFormer, 200–600 MB) | local (GPU strongly recommended) | local WebGPU if the device can handle it, else unavailable | Optional remote backend |
| Transcription (basic-pitch) | local | local | Small model, Apache-2.0 |
| Denoise (DeepFilterNet3, 8 MB) | local, tract on the CPU (vendored `deep-filter-rt`) | unavailable (no runtime yet, §8.1) | ~50× real time on an M-series CPU |
| Generation (ACE-Step), heavy enhancement | local Python sidecar (opt-in install; MPS, else CPU) | unavailable unless the build configures a remote backend | Python-only, large memory |

The engine exposes one trait, so features never depend on where a model runs:

```rust
pub trait Inference: Send + Sync {
    fn run(&self, model: ModelId, inputs: TensorMap) -> BoxFuture<Result<TensorMap>>;
}
// impls: OrtInference (native), WebOnnxInference (JS bridge), RemoteInference (HTTP)
```

---

## 3. Repository layout (target)

```
crates/
  engine-core/     # pure Rust, no I/O, no threads. DSP, pitch, scoring, autotune, mixer graph, project model.
                   # MUST compile for wasm32-unknown-unknown. No std::fs, no std::thread, no tokio.
  engine-ml/       # Inference trait, model registry, pre/post-processing (STFT, chunking, overlap-add)
  engine-native/   # cpal I/O, symphonia decode, ort backend, file/project storage
  engine-wasm/     # wasm-bindgen API, AudioWorklet processor glue, SharedArrayBuffer rings
  ingest/          # yt-dlp/ffmpeg sidecar orchestration (desktop only, behind a Cargo feature)
apps/
  desktop/         # Tauri 2 app (src-tauri uses engine-native)
  web/             # Vite + Svelte 5 app; loads engine-wasm + onnxruntime-web; projects in OPFS
packages/
  ui/              # shared Svelte 5 components (timeline, karaoke lane, graphs)
  engine-api/      # TS types for the engine API, generated from Rust (ts-rs or specta)
services/
  ml-sidecar/      # opt-in local Python (FastAPI) sidecar for models we cannot export to ONNX: ACE-Step, etc.
models/            # model manifest (name, url, sha256, license, input spec). Weights are NOT committed.
prototype/         # main.py moves here once the new engine reaches parity
docs/research/     # longer notes, benchmarks, model evaluations
```

**Dependency rule:** `engine-core` depends on nothing platform-specific. `engine-native` and `engine-wasm` depend on `engine-core`, and never the other way round. The UI only talks to the engine through the API in `packages/engine-api`.

---

## 4. F1: Import and stem separation

### 4.1 Ingest
1. Normalize the URL. Use `yt-dlp -f bestaudio --no-playlist` to fetch the best audio-only stream (usually Opus in WebM, about 160 kbps, or AAC in M4A).
2. Decode with ffmpeg to **44.1 kHz stereo f32 WAV**, the working format for all models. Keep the original file and record its codec and bitrate in project metadata.
3. Pull metadata (title, artist, duration, chapters) from yt-dlp's `--dump-json`. Lyrics lookup (F3) uses it.
4. **Desktop only.** Desktop runs `yt-dlp` and `ffmpeg` as bundled sidecar binaries. The web build has no URL import (browsers cannot download from YouTube because of CORS and bot protection, and we do not host an ingest service). In the web build the URL field is hidden behind the `ingest` feature flag. yt-dlp breaks often, so it must update itself; pin a minimum version and offer "update extractor" in the UI.
   - **As built (sidecars):** `apps/desktop/src-tauri/sidecars.toml` pins every bundled file per Rust target triple, as a GitHub release URL plus sha256. `scripts/fetch_sidecars.py` (sharing `fetch_models.py`'s checksummed downloader) puts them in `apps/desktop/src-tauri/binaries/` (git-ignored) with Tauri's `<name>-<triple>` names. They are bundled only by `tauri build --config src-tauri/tauri.bundle.conf.json` (`externalBin` plus the ffmpeg licence as a resource), so plain and CI builds need none.

     | Tool | Source | Version | Licence | Checksum |
     |---|---|---|---|---|
     | yt-dlp | github.com/yt-dlp/yt-dlp releases (`yt-dlp_macos` universal, `yt-dlp.exe`, `yt-dlp_linux`, `yt-dlp_linux_aarch64`) | 2026.10.26 | Unlicense | the release's SHA2-256SUMS |
     | ffmpeg, ffprobe | github.com/eugeneware/ffmpeg-static release b6.1.1 (macOS arm64/x64, Windows x64, Linux x64/arm64) | b6.1.1 (its macOS binaries report FFmpeg 6.0) | FFmpeg LGPL-2.1+, static builds include GPL parts; fine for GPL-3.0 | GitHub's release-asset sha256 digests |

     FFmpeg publishes no binaries. BtbN/FFmpeg-Builds (linked from ffmpeg.org) has no macOS build and rotates its daily releases, so pinned URLs would break; ffmpeg-static is the one GitHub release that covers every target under a stable tag.
   - **Lookup** (`ingest::resolve`, an explicit `Origin`): the updated copy in app data (`bin/yt-dlp`) → the bundled sidecar next to the executable → `PATH` plus the Homebrew directories → `Missing` (Home shows why). Bundled ffmpeg/ffprobe are passed as an absolute `--ffmpeg-location` directory (never a bare name); otherwise yt-dlp finds them on `PATH`.
   - **Update extractor** (Home, under the link field; shows the yt-dlp version and origin): runs yt-dlp's own `-U`, which fetches the latest GitHub release and checks it against that release's SHA2-256SUMS. A bundled yt-dlp is copied to app data first and the copy is updated (the bundle is read-only and signed); a failed or no-op update deletes the copy, so the bundled one stays in use. A system yt-dlp is left to its package manager. UI states: Checking → Ready → Updating → Updated | UpToDate | Failed. (The planned Checking/Downloading/Verifying split happens inside `yt-dlp -U`, so the UI shows one Updating state.)
   - **Known gap:** recent yt-dlp wants a JavaScript runtime (deno) for YouTube; without one it warns that some formats may be missing. Downloads still work today (verified with only the bundled tools on `PATH`). Deno is not bundled (~100 MB more); a Homebrew deno is picked up from `PATH`.
5. Local file import (mp3/m4a/wav/flac/ogg) is always supported and works without any network.

> ⚖️ Downloading from YouTube is against YouTube's Terms of Service, and the audio is usually copyrighted. That is why ingest runs only on the user's machine, for their personal use. Keep it isolated in the `ingest` crate (a Cargo feature) so builds can leave it out entirely.

### 4.2 Separation pipeline
- **Default model (proposed):** `htdemucs_6s` (Demucs v4, MIT), which gives 6 stems: vocals, drums, bass, guitar, piano, other. Use `htdemucs_ft` when only 4 stems are needed, since it is better quality.
- **High-quality vocals:** BS-RoFormer or Mel-RoFormer checkpoints (via ZFTurbo's *Music-Source-Separation-Training*). These lead the MVSEP vocal leaderboard. Check each checkpoint's license, because they vary.
- **Best result:** a two-stage cascade. (1) A RoFormer model splits vocals from the instrumental. (2) HTDemucs splits the instrumental into drums, bass and the rest. Optionally, a karaoke model then separates lead vocal from backing vocals. F2 and F3 depend on that lead-vocal stem.
- **Chunking:** split into overlapping segments (HTDemucs trains on 7.8 s segments; use about 25% overlap with a linear or triangular crossfade). Run segments in batches, then overlap-add. Shift trick (random time shifts averaged) is optional and gives a small quality gain at about 2× compute.
- **STFT/iSTFT runs outside the ONNX graph,** in `engine-ml`, because ONNX's STFT op support is uneven, especially on WebGPU. Export the models without their spectral front-end.
- **Caching:** cache stems by `sha256(source audio) + model id + version`. Never separate the same song twice.
- **Quality:** report a reconstruction residual (mix minus the sum of stems) and per-stem energy, so the UI can mark near-empty stems such as "piano: likely absent".
- **As built (M4a):** the user picks `SeparationQuality::Fast` (HTDemucs) or `BestVocals` per import. `BestVocals` is KimberleyJSN's Mel-Band RoFormer (MIT), exported by `scripts/export_melband_roformer.py` with STFT/iSTFT outside the graph. Its chunks step by `chunk − chunk/10` (overlap of exactly one fade) instead of the reference's half chunk, which needs about 45% fewer runs. Then HTDemucs runs on mix − vocals, and its vocal output folds into Other. Desktop only, on ORT CPU at about 13 s per 8 s chunk. CoreML split the graph into ~90 partitions and never finished; int8 dynamic quantization was no faster and lost accuracy. `scripts/reference_cascade.py` checks the port against the PyTorch code.

### 4.3 Transcription (stems → MIDI), optional layer
- `basic-pitch` (Spotify, Apache-2.0, small, ONNX/TFLite) runs on each pitched stem and outputs editable MIDI regions.
- Drums: onset detection on the drum stem, with a simple kick, snare and hat classifier (spectral centroid and band energy).
- Better multi-instrument models exist (YourMT3+, MuScriptor 2026). MuScriptor's weights are **CC-BY-NC**, so offer it only as an opt-in download (§0).
- **As built (M4):**
  - **Model:** `nmp.onnx` from spotify/basic-pitch (LICENSE: Apache-2.0, Copyright 2022 Spotify AB; the weights ship in that repository), pinned to commit `fa5997a`, 230 kB, registered as `basic_pitch`. I/O was inspected with onnxruntime: input `serving_default_input_2:0 [N,43844,1]`; outputs `StatefulPartitionedCall:1` note and `:2` onset `[N,172,88]`, `:0` contour `[N,172,264]` (unused).
  - **Port:** `engine-ml::basic_pitch` ports `inference.py`'s windowing (half-overlap padding, 30-frame overlap trim, the window-offset frame times) and `note_creation.py`'s decoder (inferred onsets, onset peaks ≥ 0.5, frames ≥ 0.3, 11-frame minimum, energy tolerance 11, the melodia trick). The melodia loop walks cells strongest-first instead of repeating arg-max, which is equivalent because energy only ever drops. On the same input it gives the reference decoder's notes (19 events against its 20; the difference is frame-time rounding).
  - **Hosts:** native ort in batches of 8 windows (`engine-native::transcribe`); web onnxruntime-web (WASM EP), batching the same way, with the windowing and decoding in wasm. **Accuracy:** note F1 1.000 (onset ±50 ms, pitch exact) on a 10-note synthetic pluck melody with 1/k² harmonics. With much stronger 1/k harmonics the model also hears the 5th harmonic (+28 st), exactly as the reference does; that's the model, not the port.
  - **Drums:** `engine-core::drums`: peaks of the onset envelope (≥ 12% of the loudest, ≥ 1.5× the ±0.5 s average, 50 ms apart), each classified on one 2048-sample FFT by energy share: < 150 Hz ≥ 40% → kick (36), ≥ 5 kHz ≥ 60% → closed hat (42), else snare (38). It gets 8/8 synthetic hits right, each within 30 ms. It is blind to toms, cymbals and simultaneous hits (one class per onset).
  - **Project and files:** `Project.transcriptions` (one `Transcription { source: TrackSource, method, notes: Vec<MidiNote> }` per track, `#[serde(default)]`). `midi::method_for` picks drums vs basic-pitch for both hosts. `midi::smf` writes type 0, 480 PPQ, at the detected tempo, drums on channel 10.
  - **UI:** Mix view "MIDI" per track, a per-track `MidiJob` (Idle | Transcribing | NeedsModel | Failed), a dot-grain lane under the track (pitch-spread; drums on hat/snare/kick rows), Redo, and Export .mid (desktop save dialog; web download). `TranscribeOutcome::NeedsModel` points to the Models panel.

### 4.4 Analysis extracted on import (used by every feature)
- **Tempo and beat grid:** onset envelope, then autocorrelation or tempogram, then dynamic-programming beat tracking (Ellis 2007). Downbeats come from a beat-tracking model or a heuristic.
- **Key and scale:** chroma (CQT) from the *instrumental* stem, correlated against Krumhansl–Schmuckler profiles for all 24 keys. Store the top 3 keys with confidences, plus key changes over sliding windows. Auto-tune (F4) uses this key.
- **Reference vocal pitch track:** RMVPE (built for vocals in polyphonic music) on the lead-vocal stem, 10 ms hop. Output: f0 (Hz), voicing probability and periodicity.
- **Reference note sequence:** see §6.2.

### 4.5 Timeline
- The project model is tracks, then clips (audio or MIDI), then automation. It is serializable (serde, MessagePack on disk, JSON for debugging).
- The mixer graph runs in `engine-core`: per-track gain, pan, mute/solo, a few insert FX slots, and a master bus with a limiter.
- Waveform peaks are precomputed at several zoom levels (min/max pyramids) so the UI never scans raw samples.

---

## 5. F2: Performance scoring (offline)

**Input:** the user's recording, either imported or recorded in the app. **Reference:** the lead-vocal stem's f0 and note sequence.

1. **Pre-process the user's take.** High-pass at 70 Hz and normalize loudness. If the take was recorded over speakers, the backing track bleeds into the mic; run a vocal-isolation pass on the take, or warn the user.
2. **Pitch track** with the same estimator as the reference (RMVPE), 10 ms hop. Never compare outputs from different estimators.
3. **Alignment:**
   - Recorded in the app against the backing track: the offset is known from the round-trip latency measurement (§9), so align directly. (Implemented: takes are stored on the song timeline, so scoring uses offset 0 plus the DTW below.)
   - Imported take: cross-correlate onset envelopes for a coarse offset. Then run **DTW** on chroma or f0 features with a tight Sakoe-Chiba band to absorb tempo drift. Report timing deviation separately; do not let DTW hide it.
4. **Per-frame metrics** (voiced frames in both tracks only):
   - Pitch error in cents: `1200·log2(f_user/f_ref)`. Optionally fold octaves (`mod 1200` into ±600) so a man singing a female part an octave down is not penalized. This is a user setting, on by default.
   - Hit: |error| ≤ tolerance (difficulty presets: 50, 35 and 25 cents).
5. **Per-note metrics** (using the reference note segmentation): mean cents error, % of note duration hit, onset timing error (ms), pitch stability (standard deviation of cents within the note, excluding detected vibrato), vibrato rate and extent (fit a 4–8 Hz sinusoid to detrended f0).
6. **Scores (0–100):** pitch accuracy, timing, stability, and coverage (% of reference voiced time the user also sang). The overall score is a weighted sum, with the weights in a config file. Keep the formulas documented and deterministic; never use an ML "vibe score".
7. **Visualization:** reference notes drawn as bars on a piano-roll background, the user's f0 as a line colored by error (green, yellow, red), a cents-error strip below, and per-section score cards. Clicking a note plays that section of both takes A/B.

---

## 6. F3: Live karaoke

### 6.1 Lyrics (fetched on demand)
1. **LRCLIB** (`https://lrclib.net/api/get?artist_name=&track_name=&album_name=&duration=`) is free, needs no API key, and returns line-synced LRC. Fall back to `/api/search`. Cache results locally. Send a proper User-Agent naming the app, as LRCLIB asks.
2. Line timing → **word timing.** Run forced alignment on the separated vocal stem: CTC forced alignment (a wav2vec2-style acoustic model plus the known lyric text) inside each LRC line window. If no synced lyrics exist, transcribe with Whisper word timestamps and fuzzy-match against the plain lyrics. Check model licenses: Meta's MMS aligner weights are CC-BY-NC, so the default is an MIT or Apache aligner, or Whisper (MIT). MMS is available only as an opt-in download.
3. Fetch only when a song enters karaoke mode, never during import. The user can manually nudge the global offset (±ms) and edit individual lines.
4. Do not bundle or redistribute lyric text. Fetch it at runtime and cache it per user.

### 6.2 Reference notes (the bars in the karaoke lane)
Convert the reference f0 track into discrete notes, in the style of UltraStar or SingStar:
- Median-filter f0 (about 50 ms window), convert to fractional MIDI pitch, and drop frames below the voicing threshold.
- Segment with an HMM, or with a simpler change-point rule: start a new note when the pitch moves more than 70 cents from the running note median for at least 60 ms, or at a voicing gap of at least 40 ms.
- Snap each note to the nearest semitone, biased toward the detected key's scale. Merge notes shorter than 80 ms into their neighbors.
- Optionally use word boundaries from the lyrics alignment as extra split points, so each syllable gets its own note.
- Save the notes as editable MIDI on a hidden "Reference Vocal" track.

### 6.3 Live pitch path (latency-critical)
- Mic input, then a ring buffer, then a pitch estimator, then UI updates at 60 Hz.
- **Estimator:** start with **YIN/pYIN in Rust** (cheap, predictable, about 1 frame of latency). Use a 2048-sample window (~46 ms at 44.1 kHz) with a 256–512-sample hop, sized for the voice range (65–1100 Hz). Add **SwiftF0** (~96k parameters, ~42× faster than CREPE on CPU) through ONNX as the robust option. The live path must not depend on GPU inference.
- **Target end-to-end display latency is ≤ 80 ms.** Audio scheduling must be sample-accurate. The UI only interpolates between engine timestamps; it never decides musical time.
- **Bleed:** the mic hears the backing track unless the user wears headphones. Default flow: strongly recommend headphones. Fallbacks are browser `echoCancellation: true`, or our own AEC using the known backing signal as the reference, which is a later feature.
- **Live scoring** uses §5's per-frame rules, with lag compensation: measured input latency is subtracted before comparing against the reference.

### 6.4 Display
- A horizontal scrolling lane. The y-axis is semitones, auto-ranged to the song's vocal range, with octave folding on by default. Reference notes are bars. The user's pitch is a trailing line or blob. A note fills in as the user hits it, and a combo or streak counter runs alongside.
- Two lyric lines, with word-level karaoke wipe on the current one.
- Render with Canvas or WebGL in one requestAnimationFrame loop. Never create DOM elements per note.

### 6.5 As built (M2)
- **Lyrics** (`engine-core::lyrics`, `packages/ui/src/lyrics.ts`, `LyricsPanel.svelte`):
  - Looked up the first time a song opens in Karaoke, never at import: `/api/get` (artist, title, rounded duration), then `/api/search`, preferring synced results and then the closest duration. Instrumental results are skipped.
  - Only the artist, title and duration are sent. Browsers can't set `User-Agent`, so both hosts name the client in the `Lrclib-Client: Grain/0.1` header, which LRCLIB accepts and allows through CORS. The lookup runs in the shared UI with `fetch` (the desktop webview has no CSP), so there's one code path.
  - Artist and title are guessed from the video title or the file name (`Artist - Title`, `artist-title`) and can be edited before searching again.
  - The result is stored on the project (`Project.lyrics: ProjectLyrics`, NotFetched | NotFound | Plain | Synced), so it is "cached" in the desktop cache entry's `project.json` and in the web OPFS `.grain`. Lyric changes mark the session unsaved, like a mix change; the explicit Save persists them. Text lives only in the user's own project.
  - UI states: Idle → Fetching → Synced | PlainOnly | NotFound | Failed (Retry). Plain-only lyrics show as static text, with no wipe.
  - LRC parsing: `[offset:]` tags (positive means earlier, per the format), several timestamps on one line, metadata tags skipped, `<mm:ss.xx>` word tags stripped. The user's offset nudge is ±100 ms per step (positive means later). Per-line edits change text and start time. Times are shown and edited with the offset applied.
- **Word timing, UltraStar-style** (`lyrics::split_units`, `time_syllables`): a line splits into sung units (one per CJK character, with small kana, `ー` and closing punctuation attached; one per Latin word). The units are spread over the reference notes that start inside the line, in order: with fewer units than notes, each unit takes a run of notes; with more, a note's time is shared. With no notes, the line's time is shared evenly. **ML forced alignment (§6.1.2) is a later upgrade**: it needs an MIT/Apache aligner model and only improves the wipe, not scoring.
- **Live pitch** (`engine-core::live`): `LivePitch` runs the offline YIN tracker incrementally (same window, hop and voicing via the shared `pitch::frame_pitch`), and `LiveSession` compares each frame with the reference using the shared `score::compare_frame`, keeping `FrameTotals`. A test proves the live frames and verdicts equal the offline tracker's and scorer's. The live path is YIN on CPU on both hosts, even when the reference was tracked by RMVPE.
  - Desktop: the capture writer thread's growing buffer is read by the `live_pitch` command (never the audio callback). The song time of a capture sample comes from the same cue and first-capture instants and the latency that `finish_recording` uses. Web: `WasmLive` is fed from the same SAB ring drain as the take.
  - Latency: frames are placed on the song timeline with the stored round-trip latency already removed. The trail head shows up to one YIN window (46 ms) plus a UI frame behind the playhead, within the 80 ms target, because it's drawn where it was sung rather than at the playhead.
- **Lane** (`KaraokeView.svelte`, `grain.ts` `PitchTrail`, the one class also used by the Score preview): the y-axis is semitones over the song's note range ±2, and the trail is folded to the octave of the note being sung when octave folding is on. Grains are placed at their song time and scroll with the notes. A note fills hit, near or miss (`feedback.ts`, the same thresholds as the Score graph) once the song passes it; hits burst and add to the streak. The running score is the share of the reference's voiced frames sung in tune (`inTune` = hit / (hit + miss + reference only)).
- **Session** (`karaoke.ts`): Idle → CountIn → Singing ⇄ Paused → Finished, plus Failed. Each uninterrupted run (count-in to Pause or Finish) is recorded, saved as a take and scored offline (`microphone.finish`), so pausing never loses singing and the takes show in Score. Resume counts in again from the paused position. Finish during a count-in drops that empty run. Leaving the view mid-run finishes it.
- **Shared with Score:** `SettingsControls` (difficulty and octave, kept on the session), `RecordingSetup` (the backing choice, latency nudge and calibration), and the `recorder.ts` arm/finish/cancel helpers.

---

## 7. F4: Auto-tune (DSP)

**Signal flow:** detect f0 → choose a target pitch → smooth it → pitch-shift by the ratio.

1. **Detection:** the same YIN estimator as §6.3 for live use, RMVPE for offline.
2. **Target selection:** quantize to the nearest note in the active scale. The key and scale are auto-detected (§4.4) and can be overridden. Modes: chromatic, major/minor, custom, and "follow reference melody", which snaps to the reference note at that moment, a karaoke-specific mode. Hysteresis of about 15 cents at note boundaries stops the output flickering between two notes.
3. **Retune speed:** smooth the correction with a one-pole filter on the target. Time constant 0 ms gives the hard "T-Pain" effect; 50–400 ms sounds natural. **Humanize** reduces correction on sustained notes and leaves the natural vibrato in. Unvoiced frames (consonants, breaths) pass through uncorrected.
4. **Shifter:**
   - **Live:** **TD-PSOLA**, our own implementation. It preserves formants automatically, adds roughly one pitch period plus the analysis window of latency, and costs little on the CPU. Place pitch marks with the YIN period and a peak-picking pass.
   - **Offline high quality:** a phase-vocoder or spectral shifter with formant preservation (spectral envelope from cepstral liftering), or WORLD-style analysis and synthesis. Signalsmith Stretch (MIT) is the quality benchmark; we can use it natively through a `-sys` crate, and its official Web Audio/WASM build works for web prototypes. Rubber Band (GPL) is allowed because the project is GPL. It is native-only; check whether it builds for wasm before relying on it on the web.
5. **Parameters exposed:** key, scale, retune speed, humanize, formant shift, mix (wet/dry), and an editable correction curve ("graphical mode", later).

### 7.1 Pitch editor (Melodyne-style, simplified), milestone M4

Offline, note-based editing of a **monophonic** vocal (the user's take or the vocal stem). It's less sophisticated than Melodyne: no polyphonic (DNA) editing and no amplitude or sibilant tools, but the core workflow is the same.

1. **Analysis:** RMVPE f0 track plus note segmentation, reusing §6.2's segmentation code (one implementation, no copy). Each note is a "blob" that stores start, end, its pitch centre (median cents), drift (a slow curve), modulation (vibrato, the f0 detrended against drift), and its amplitude envelope for drawing.
2. **Display:** a piano roll in the Pitch view. Blobs are drawn as dot-grain shapes in the stem's hue, sized by amplitude. The continuous f0 line sits on top, and scale lines come from the detected key.
3. **Edits** (all non-destructive; stored as an edit list in the project):
   - Drag a blob up or down: snap to a semitone or to the scale; Alt-drag moves it freely in cents.
   - Drag it left or right, or drag its edges: move the note in time or stretch it.
   - **Pitch centre** tool: % correction toward the target note, applied to the selection or the whole take.
   - **Pitch drift** tool: % flattening of the slow drift inside a note.
   - **Modulation** tool: % scaling of vibrato, from 0% (flat) up to 200%.
   - **Split / merge** notes; **transition** handles that set how fast the pitch glides between neighbouring notes.
   - **Formant** offset per note.
   - Select all and "Correct pitch" applies centre plus drift correction in one step.
4. **Rendering:** turn the edits into a time-varying pitch-ratio curve (plus a time map for timing edits), then run M3's offline high-quality shifter with formant preservation. Re-render only the edited regions, and crossfade at their boundaries.
5. **Shared code:** the target-note and scale snapping is the same code as auto-tune §7 step 2. Live auto-tune is just this editor's "Correct pitch" applied in real time with a smoothing filter, so the two share one engine module.

### 7.1.1 As built (M4)
- **Edit list** (`engine-core::edit`, kept as `Project.pitch_edits`, one `PitchEdit` per `EditSource` (Take | VocalStem), `#[serde(default)]`):
  - Each `EditNote` holds its sung span and centre (median voiced pitch), and what changed: `pitch` (target centre), `drift` (share kept), `modulation` (0–2), `transition` (glide seconds), `formant`, and `out_start`/`out_end` (timing).
  - `analyze` = §6.2's `melody::segment` + medians; snapping is `tune::Scale::snap`, with the scale defaulting to auto-tune's (`Project.tune.scale`).
  - `EditOp` lists every edit: MovePitch (Free | Semitone | Scale snap), Correct (centre %, drift %), SetModulation, SetTransition, SetFormant, MoveTime, Stretch, Split, Merge, Reset, SetScale. Unknown ids do nothing; time edits clamp between the neighbours, to 0.5–2× and ≥ 50 ms.
- **Curves:** per sung frame inside a note, `offset + (drift−1)(d−centre) + (modulation−1)(m−d)`. Here `d` is the ±75 ms moving average of the note's voiced pitch (drift) and `m` the sung pitch, so `m − d` is the vibrato. The offset ramps over `transition` from the previous note's offset (if that note ends < 0.1 s before) or from 0. The time map anchors `(out_start→start, out_end→end)` per note plus the song ends.
- **Rendering:**
  - `stretch::warp` (WSOLA, 23 ms Hann frames, ±¼-frame search) re-times only the changed stretch and copies the rest bit-for-bit, with 10 ms crossfades.
  - The curves are moved onto the output timeline, then `shift::shift_curves` (the M3 phase vocoder with a per-frame formant curve) runs only on spans with a non-zero curve (±100 ms), with 20 ms crossfades.
  - **Accuracy:** a +2 st note lands within 5 ¢ (the test passes); the untouched note is **bit-identical**.
  - The result is a new take, `TakeSource::Edited { from }`, "<source> (edited)", re-tracked and scored like a tuned take.
- **Hosts:** `PitchEditor` (`open` / `apply` / `set` / `fromTune` / `render`) over shared project-level helpers (`edit::open`, `apply_op`, `store`, `seed_from_tune`, `render_source`). Desktop decodes just the take or the vocal stem (`StemCache::track_audio`); web uses the song it holds.
- **UI** (`PitchView.svelte`, state in `pitch.ts`):
  - **State machines:** `Loading` (Loading | Ready | Failed); `Selection` (None | Notes); `Drag`: Idle | Pressing (direction undecided until 5 px) | Pitch | Time | Stretch | Panning (with rubber band, rule 19). While held, the blob follows the pointer directly (rule 14); the engine applies the op on release. `RenderState`: Idle | Rendering | Rendered | Failed.
  - **Undo/redo:** `History` keeps snapshots of the edit list (100 steps; a slider drag coalesces into one step by key); `set` hands a restored list back to the engine.
  - **Drawing:** blobs are dots along the note line, rows by loudness (`edit::amplitude`), with scale rows from the engine's `scale_mask`.
  - **Keys:** ⌘Z/⇧⌘Z (and ⌘Y), ⌘A, Delete/Backspace resets the selected notes (it doesn't remove them, so the audio isn't cut), Esc.
  - **Source:** the newest original take by default, else the vocal stem; the source stays selected after rendering.
- **M3's editable correction curve:** Auto-tune's "Edit curve…" calls `edit::from_tune`. It seeds the take's edit list with each note's centre moved to the target auto-tune would pull most of its frames to (× mix) and the formant setting, then opens it in the Pitch view. One mechanism, no second curve editor. Retune speed and humanize aren't carried over: in the editor, glides and drift take their place.

### 7.2 As built (M3)
- **Target selection** (`engine-core::tune`, shared by live, offline and M4): `Scale` (Chromatic | Key | Custom pitch-class mask) with `snap`; `TuneMode` (Scale | FollowMelody, the reference note in the singer's octave, the scale where the melody rests); `Tuner`, one frame at a time, allocation-free, with an explicit `Target` (Free | Holding) and 15 ¢ hysteresis (`HYSTERESIS`). Retune is a one-pole filter on the *correction* (target − sung), so vibrato faster than the time constant survives; 0 ms snaps. Humanize lengthens the time constant by up to 400 ms as a note is held (full after 0.25 s). Unvoiced frames reset it and pass through. `correction_curve` runs the tuner over a take; `preview` gives the graph's sung, target and corrected curves; `render` makes the tuned take.
- **Settings live on the project** (`Project.tune`, the detected key at import; `#[serde(default)]` chromatic for older projects), so they save with the song. The hosts' `setTune` updates the project and the live tuner together.
- **Live: our TD-PSOLA** (`engine-core::psola`): `Psola` puts analysis marks one YIN period apart, at the largest sample within ±¼ period (peak picking), and a fixed period while unvoiced. Grains are Hann, asymmetric (previous period up, next period down), so at ratio 1 the output rebuilds the input (SNR 146 dB). Synthesis marks are spaced period ÷ ratio, each taking the nearest mark whose grain has fully arrived; it never waits, so playback never misses a grain. A formant shift resamples the grains. `LiveTune` = `pitch::frame_pitch` (the offline tracker's window, hop and voicing) + `Tuner` + `Psola` + dry/wet mix with the dry voice on the same delay. Everything is preallocated; `process` doesn't allocate or lock.
  - **Latency:** fixed at (formant max + 1.5) × the longest period (90 Hz) = **31.5 ms** at 44.1 kHz, plus the devices' round trip. The UI shows the total (`MonitorStatus`: calibrated round trip + shifter delay).
  - **CPU:** 0.3% of one core at 128-frame blocks (release benchmark `psola::tests::live_cpu_under_a_quarter_core`; budget 25%).
  - **Accuracy:** ±1/±3/±12 st on a synthetic sung vowel within 0.1 ¢. Formant error (the harmonics against the vowel's true resonances) is 1.4 dB, against 7.9 dB for plain resampling at +3 st.
- **Monitor in the player:** `PlayerCommand::Monitor { mode: Off | Dry | Tuned }` and `Tune { settings }`. `Player::render(left, right, input)` takes the mic block; the tuner keeps running in Dry, so switching is seamless. Dry adds no delay.
  - Desktop: `capture::MonitorInput` opens the input at the *output's* rate (it fails with the reason if the device can't, which shows as the UI's Failed state). It pushes mono into an `rtrb` ring handed to the playback callback through a feed ring; replaced rings go back through a `retired` ring, so the callback never frees one. Queued input beyond one block + 512 frames is skipped, so clock drift can't build up delay. `Capture` and `MonitorInput` share one `InputStream` (thread-owned cpal stream).
  - Web: the worklet copies `inputs[0][0]` into the player's input buffer each quantum (same clock, no resampling). Takes, calibration and the monitor share one mic connection (`MicInput`: Closed | Opening | Open, with holders), so the input never carries the voice twice.
- **Offline shifter: our own phase vocoder** (`engine-core::shift`) on `Stft::map`, a streaming forward → edit → inverse sharing `Stft`'s analysis and overlap-add code, so a long take never holds all its frames. Each frame is whitened by its spectral envelope; partials move to `k · ratio` with peak-locked phases (Laroche–Dolson identity locking, done in the zero-phase frame (`+πk`), so a stretched main lobe stays one coherent sinusoid); then the envelope, optionally formant-shifted, goes back on. Frames with no correction pass through bit-exact.
  - **Envelope:** the cepstrum finds the pitch period, and the envelope is the log-magnitude running maximum over one harmonic spacing, smoothed. This passes through the harmonic peaks. A plain liftered cepstrum averaged peaks with valleys and was 2× worse in the formant test.
  - **Accuracy:** ±1/±3/−12 st within 0.02 ¢, +12 st −3.7 ¢. Formant error 1.8 dB (resampling: 7.9 dB). A +3 then −3 st round trip: formant error 2.6 dB, log-spectral distance 2.6 dB. About 190× faster than real time (release).
  - **Why not Signalsmith or Rubber Band:** Signalsmith Stretch is C++; it would need a C++ toolchain in the wasm32 build, and it time-stretches rather than taking a per-frame pitch curve. Rubber Band is native-only. Ours is pure Rust on the existing STFT, so desktop and web run the same shifter, and M4's editor can drive it with any curve.
- **Tuned takes:** "Apply to take" renders `tune::render` (correction curve → shifter → dry/wet) and adds a new take with `TakeSource::Tuned { from }`, named "<take> (tuned)". It is re-tracked with the project's tracker, scored, saved like any take (desktop: cache `take-<id>.wav`, read back with `StemCache::take_audio`; web: in the `.grain`), and the original is untouched.
- **View** (`AutoTuneView.svelte`, `TuneGraph.svelte`, `autotune.ts`):
  - Controls: Key (the analysis's keys, Chromatic, or Custom notes on a one-octave keyboard), Pull to (scale or melody), Retune, Humanize, Formant, Mix, and the monitor toggles with the headphones hint.
  - Monitor state: Off → Switching → On(status) | Failed(message). Leaving the view turns it off.
  - Take picker, Apply state Idle → Applying → Applied | Failed, and a "Hear" A/B (the song, the original take, or the tuned take in place of the vocal; `listenCommands`).
  - The graph shows the sung pitch, target notes and corrected pitch from `preview` before rendering.

---

## 8. F5: AI production

| Task | Approach (proposed) | License note |
|---|---|---|
| Denoise (live and offline) | **DeepFilterNet** (has a Rust implementation; real-time on CPU; 48 kHz) | MIT/Apache |
| De-reverb, restoration, bandwidth extension | Resemble-Enhance or a similar speech/vocal enhancer (server) | check per model |
| Vocal chain (DSP, ours) | HPF → de-esser → compressor → EQ → saturation → reverb/delay, with presets per genre. Presets are chosen by measuring the vocal (loudness, spectral tilt, sibilance energy) | n/a |
| Generate backing track from a vocal | **ACE-Step 1.5** (MIT, verify the weights license before integrating): vocal-to-BGM, "lego" (add a single instrument track), repaint, extract. Conditioned on the vocal stem plus a text prompt plus the detected tempo and key. Runs as an opt-in local Python GPU sidecar, installed by the model manager. The lego and complete tasks reportedly need the XL-base checkpoint. | MIT |
| Melody / harmony | Rule-based first: harmonies at diatonic thirds and sixths from the reference notes, pitch-shifted with §7. Then model-based (ACE-Step, or a MIDI model generating into the timeline) | |
| Missing instruments | ACE-Step lego conditioned on the existing stems. Results land as new audio tracks on the timeline | |
| Voice conversion (later) | RVC-style models | Only on the user's own voice, or with consent |

Generated audio always lands as **new, non-destructive tracks** and never overwrites anything. Store the model, the prompt and the seed with each generated clip so results can be reproduced.

### 8.1 As built (M5)

A **Produce** session view works on a *source* (a take or the vocal stem, the pitch editor's
`EditSource`). Every result is a new take with a `TakeSource` that records how it was made:
`Denoised { from, model }`, `Chained { from, preset, settings }`, `Harmony { from, interval }`,
`Generated { request }` (task, prompt, seed, model, bpm, key). Sung results (cleaned, chained)
start muted and are scored like any take; harmony and generated tracks start audible with level and
pan defaults (`TakeSource::is_sung`). The view lists everything it made with play/level/pan. The
host contract is `EngineHost.producer`.

- **Denoise (DeepFilterNet3).** `deep-filter-rt` 0.2.0 (DeepFilterNet's own inference code,
  MIT OR Apache-2.0) is vendored in `third_party/` with two changes (its `NOTICE.md`): the embedded
  model is removed and `rust-version` lowered to 1.95 (it declares 1.96 but builds unchanged). The
  model manager downloads upstream's `DeepFilterNet3_onnx.tar.gz` (pinned commit `d375b2d`,
  sha256-checked, from raw.githubusercontent.com); `engine-native::denoise` reads its four members
  straight from the archive (no extraction step in the manager). Upstream's README states code
  *and* weights are MIT OR Apache-2.0. Takes are resampled to 48 kHz with rubato, processed in
  10 ms hops, the 30 ms model delay is removed, and the result is resampled back; the attenuation
  limit is 30 dB (full suppression sounds processed on singing). Measured on synthetic speech-like
  audio in white noise (4 s, 44.1 kHz): **SNR 8.6 → 21.3 dB (+12.7 dB)** at **~0.02× real time**
  (release build, M-series CPU). **Desktop only:** tract adds ~25 MB to a wasm build and the
  ONNX graphs need DeepFilterNet's own pre/post-processing, so the web reports Unavailable.
  **No live denoise:** tract allocates per inference, which §9 forbids on the audio thread.
- **Vocal chain** (`engine-core::chain`, pure DSP, desktop and web): 24 dB/oct high-pass →
  de-esser (a 6 kHz high-passed sidechain ducks the whole voice; a band split smeared phase) →
  feed-forward compressor (5/100 ms) → low shelf 200 Hz, presence peak 3 kHz, air shelf 10 kHz
  (RBJ biquads) → tanh saturation → Freeverb-style reverb and a feedback delay (a dotted eighth at
  the song's tempo). `VocalChain` allocates its lines in `new` and nothing per block, so the player
  runs it on the live monitor after auto-tune (`PlayerCommand::Chain { settings | null }`);
  `render` runs it offline. `measure` reads integrated loudness (BS.1770-4 K-weighting with gates;
  a −20 dBFS-peak 1 kHz sine reads −23.0 LUFS), sample peak, tilt (2–6 kHz over 150 Hz–1 kHz) and
  sibilance (5–10 kHz over 150 Hz–5 kHz). `settings(preset, measures, bpm)`: every preset sets
  the compressor and de-esser thresholds from the take's loudness, and *Auto* additionally moves presence and air halfway towards a neutral tilt and
  scales de-essing with the measured sibilance. `produce::pick` then sets the output gain
  from a trial render, so every preset lands on −16 LUFS (within 0.5 LU in tests; the gain is capped
  at ±24 dB). Tests cover every stage, block-size independence,
  the LUFS reference, and Auto on synthetic bright/dark/quiet/loud voices. The whole chain costs
  **~0.34% of one core** in 128-frame blocks (release, `chain_cpu`), well inside §10's live budget.
- **Harmony** (`engine-core::harmony`): diatonic thirds and sixths (above and below) in the
  song's key, per note of the source's melody (the reference notes for the vocal stem, segmented
  notes for a take); out-of-scale notes keep their chromatic offset. The per-frame correction is
  the note's interval, so the singer's inflections ride along, rendered by the formant-preserving
  shifter (§7.2) with formant shift 0. Harmony tracks start at 60% level, panned ±0.35.
  **Melody MIDI** exports the source's notes (or the harmony's) through `midi::smf_notes`.
- **ACE-Step 1.5 sidecar** (`services/ml-sidecar/`). Code: github.com/ace-step/ACE-Step-1.5 (MIT,
  pinned `ca1e85f`); weights on Hugging Face tagged MIT (`acestep-v15-base`, `Ace-Step1.5`). Every
  task uses the **2B base** DiT: per ACE-Step's model zoo only *base* models support `lego`,
  `complete` and `extract` (turbo and SFT don't), and XL-base (4B) needs ≥ 20 GB, so the earlier
  "XL-base is required" note was wrong: any base model works. Tasks: vocal-to-backing = `complete`,
  one instrument = `lego` (ACE-Step's track names), `repaint` over the song's mix between two
  times. The 5 Hz LM is not used (`thinking=False`), so its weights aren't downloaded.
  - **Install** (Models panel, opt-in "Install AI generation (several GB)"): copies the service
    into app data, `uv sync --frozen --extra acestep` (PyPI + the pinned git source; the lock is
    committed), then `python -I -m grain_sidecar download` (Hugging Face: base DiT 4.79 GB, VAE
    0.34 GB, Qwen3-Embedding-0.6B 1.19 GB = **6.3 GB**). Python env with PyTorch ≈ 2 GB on macOS.
    Needs `uv` on PATH (or Homebrew/`~/.local/bin`); it is not bundled.
  - **Lifecycle** (`engine-core::generate::GeneratorState`): NotInstalled → Installing{step} →
    Ready → Running → Ready; Failed{message, installed} (a failed run can retry, a failed install
    must reinstall); Uninstalled → NotInstalled. The sidecar is started for one generation
    (the venv's `python -I -m grain_sidecar serve`, not under `uv run`, so killing the child stops
    it), bound to 127.0.0.1 on a free port, with a random 128-bit bearer token, and killed when the
    generation ends: no Python stays resident.
  - **HTTP contract** (`grain_sidecar/server.py`): `GET /health`; `POST /generate?task&prompt&seed&
    bpm&key&duration[&track][&start&end]` with the conditioning WAV as the body, answering a WAV.
    The query is built by `GenerationRequest::query` and WAVs by `engine-core::wav`, shared by
    desktop and web. Results are resampled to the project rate and kept as one mono track.
  - **Web:** Unavailable unless built with `VITE_GRAIN_GENERATION_URL` and
    `VITE_GRAIN_GENERATION_TOKEN` (a sidecar run elsewhere; same contract).
  - **Verified without the model:** pytest against the stub backend (token, WAV in/out, request
    validation) and a Rust test that starts the real service with the stub backend through the
    same spawn code, checks the token, runs complete/lego/repaint, passes an error through, and
    checks the port is closed after stop. ACE-Step itself (weights, real generation, MPS speed and
    memory) was **not** run during development.
- **De-reverb: deferred.** No candidate has both a GPL-compatible licence and a light ONNX path:
  - anvuew's de-reverb Mel-Band RoFormer (GPL-3.0, 913 MB; a 439 MB 256-dim variant). It is our
    RoFormer architecture, so the export script and `engine-ml::roformer` should carry it, but it
    is as slow as Best vocals on CPU. This is the likely next step, as an opt-in local build.
  - Resemble-Enhance (MIT code and weights) is PyTorch/DeepSpeed only, so it would be another
    sidecar task.

---

## 8.5 UI motion rules (mandatory for all UI work)

All durations, easings and transitions live in **one** module, `packages/ui/src/motion.ts` (tokens + Svelte transition/action helpers), so the no-duplication rule applies here too. Components never hard-code a duration or easing. Visual design is still to come; these rules apply regardless of the look.

| # | Rule | How |
|---|------|-----|
| 1 | **Ease-out for entrances** | Anything appearing uses an ease-out curve (`cubicOut`/`quartOut`). |
| 2 | **Dropdowns at 150–250 ms** | Menus, selects and popovers open in 150–250 ms (token `dur.menu` = 200 ms). |
| 3 | **Stagger list rows by 30 ms** | Lists (tracks, stems, songs, score rows) enter with `delay = index × 30 ms`, capped so a long list finishes in under 300 ms. |
| 4 | **Scale to 0.97 on press** | Pressable elements scale to 0.97 while `:active` / pointer down. |
| 5 | **Stretch moving knobs 12%** | Slider and toggle thumbs stretch up to 12% along the direction of travel while moving, then settle back. |
| 6 | **Stagger as details settle** | When a panel opens, its container comes in first, then its inner details follow in a short stagger. |
| 7 | **Arcs for object movement** | Objects that travel across the screen (e.g. a clip dropped onto another track) follow a slight curved path, not a straight line. |
| 8 | **Pulse to confirm success** | Completed actions (stems ready, export saved, note hit) get one brief scale/opacity pulse. |
| 9 | **Ease-in-out for moves** | Elements already on screen that move or resize use ease-in-out. |
| 10 | **Start popovers at 0.95** | Popovers and menus enter from `scale: 0.95, opacity: 0`. |
| 11 | **Enter from the trigger** | Popovers and menus scale from the element that opened them (`transform-origin` set to the trigger side). |
| 12 | **Exit to the trigger** | They also close back toward that trigger. |
| 13 | **No bounce on everyday UI** | No overshoot or spring bounce on routine controls. Bounce is only for rare celebratory moments (e.g. the final score reveal). |
| 14 | **Don't tween while held** | During a drag or scrub (knobs, faders, playhead, clip drag), the element follows the pointer directly with no animation. Animation applies only after release. |
| 15 | **Make exits 20% shorter** | Exit duration = entrance duration × 0.8, derived from tokens and never written separately. |
| 16 | **Overlap old & new states** | State swaps (views, tabs, content) crossfade with old and new overlapping, never out-then-in with a blank gap. |
| 17 | **Masks for reveals & colour** | Reveals (waveform drawing in, karaoke lyric wipe, colour changes) use clip-path or mask wipes rather than fades. |
| 18 | **Crossfade icons + blur** | Icon swaps (play ↔ pause, mute on/off) crossfade with a slight scale and a 2–4 px blur. |
| 19 | **Rubber band past bounds** | Dragging or scrolling past a limit (timeline start, fader max, zoom limits) gives with damped resistance, then springs back on release. |
| 20 | **Reuse the same layoutId** | An element that appears in two places (song card → open project, clip → editor) animates between them as one shared element: Svelte `crossfade` with a shared key, or FLIP. Never fade one out and the other in. |

**Also:**
- Respect `prefers-reduced-motion`: collapse animations to opacity-only or instant.
- Animate only `transform`, `opacity`, `filter` and `clip-path`; never layout properties.
- Canvas views (timeline, karaoke lane) follow the same tokens in their own render loop. Rule 14 always applies to playhead scrubbing.

---

## 9. Real-time audio rules (non-negotiable)

On the audio thread (cpal callback / AudioWorklet `process()`):
- **No allocation, locks, syscalls, logging or panics.** Preallocate everything. Use `rtrb` or SPSC rings for audio data and lock-free triple buffers for parameters and UI snapshots.
- **No ML inference** on the audio thread. Inference runs on a worker thread, and results come back through rings.
- Block sizes vary. AudioWorklet's render quantum is 128 frames; do not assume more. Desktop targets 128–256 frames.
- **Latency calibration:** measure round-trip latency with a loopback chirp test (the Score view's Calibrate), and store it per input/output device pair. Recordings are compensated automatically.
- **Known exception (web):** the worklet's message handler copies a song's tracks, or a new take, into wasm memory when it arrives (`load`, `addTrack`). That allocates on the audio thread between render quanta, not inside `process()`. Desktop hands takes to the callback pre-built through a ring and keeps the audio alive on the control side.
- Use `f32` everywhere in DSP. Keep the internal sample rate the same as the device rate; resample files on load with `rubato`, never in the callback.
- **Web:** `SharedArrayBuffer` needs cross-origin isolation, so the web host must send `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp`.

---

## 10. Engineering conventions

- **Testing:**
  - DSP code gets golden-file tests: synthetic signals (sines, chirps, vibrato, PSOLA round-trip) with tolerances in cents or dB, not exact equality.
  - Pitch estimators are benchmarked on MIR-1K / PTDB-TUG excerpts kept in `docs/research`.
  - Separation is checked with SDR on a handful of MUSDB18-HQ tracks (not committed; a download script is fine).
  - Every `engine-core` test must also pass under `wasm32` (`wasm-pack test --node` or `wasm-bindgen-test`).
- **Determinism:** the offline analysis code (scoring, note segmentation, key detection) must give identical output for identical input on every platform. No `HashMap` iteration order in outputs, and seeds are fixed.
- **Models:** each model is registered in `models/manifest.toml` with its source URL, sha256, size, **license**, input and output spec, sample rate, and a `distribution` field (`default`, or `opt-in` for NC weights). CI fails on an unknown license, on an NC model marked `default`, or on a downloadable file without a sha256. A `[model.build]` table marks locally exported models; `manual = "…"` marks models that can't be fetched (and says why).
  - **Model manager (M4):** `engine-ml::models` compiles the manifest in (`include_str!` + `toml`), so desktop, web and the tests read the one file. It also holds the `ModelState` machine (NotDownloaded → Downloading{received,total} → Verifying → Ready | Failed; Deleted → NotDownloaded; a retry from Failed), the streaming SHA-256 and the host allow-list (`huggingface.co`, `github.com`, `raw.githubusercontent.com`, https only).
  - Desktop `engine-native::models` downloads with ureq (native TLS) into app data as `.part` files and renames them only after every checksum matches. Web `apps/web/src/models.ts` streams into OPFS through the wasm checksum and writes a `<id>.ok` marker only after verifying. A failure deletes what was written. After a change, separation and pitch look for their weights again.
  - Opt-in models need `acceptLicense`, which the UI only passes from the licence dialog. `ModelEntry.removable` tells the app's own downloads from weights it can't delete (a development checkout).
- **Errors:** `thiserror` in libraries and `anyhow` only in binaries. Engine API errors are typed and mirrored in TypeScript.
- **Dependency licenses:** `cargo deny` checks that every crate is GPL-compatible, and `license-checker` does the same for npm packages.
- **Style:** `cargo fmt`, `cargo clippy -D warnings`, and Prettier plus ESLint for TypeScript. Public engine API types carry `#[derive(Serialize, Deserialize, TS)]`.
- **Performance budgets:**
  - Live path (pitch + autotune + mixer) under 25% of one core at 128-frame buffers.
  - 4-minute song import, desktop with a GPU: separation under 60 s.
  - Timeline steady at 60 fps with 12 tracks.

---

## 11. Roadmap

Keep this table current: update the Status column whenever a milestone moves.

| Milestone | Scope | Status |
|---|---|---|
| **M0** engine skeleton | Cargo workspace, YIN, mixer, decode, CLI with prototype parity, wasm32 build | ✅ done |
| **M1** F1 + timeline + F2 | Import (file + YouTube), HTDemucs, tempo/key, reference notes, timeline, offline scoring | ✅ mostly done (leftovers below) |
| **UI** Grain design pass | Home → Session, blended title bar, view tabs, shared particle module, desktop save/open, recents | 🔄 in progress |
| **M1 leftovers** | In-app recording, DTW alignment, RMVPE, A/B playback, web persistence, bundled sidecars, CI | ✅ done |
| **M4a** (moved up) | RoFormer vocal cascade (GPL-compatible checkpoint) for non-pop music | ✅ done (desktop, opt-in "Best vocals"; web is Fast-only) |
| **M2** F3 live karaoke | LRCLIB lyrics, word alignment, live pitch lane, latency calibration | ✅ done (UltraStar-style word timing; ML alignment later) |
| **M3** F4 auto-tune | Live PSOLA auto-tune, then the offline high-quality shifter | ✅ done (see §7.2) |
| **M4** quality + pitch editor | Melodyne-style pitch editor (§7.1), basic-pitch MIDI layers, model manager with opt-in NC downloads | ✅ done (see §7.1.1, §4.3, §10) |
| **M5** F5 AI production | DeepFilterNet + vocal chain, then the ACE-Step local sidecar | ✅ done (see §8.1; de-reverb deferred) |

### Milestone details

1. **M0, engine skeleton:** the Cargo workspace, plus `engine-core` with YIN, a WAV/symphonia loader and a simple mixer. It must run in a cpal CLI *and* an AudioWorklet demo. Port `main.py`'s comparison so the result can be checked against the prototype. CI builds for macOS, Windows, Linux and wasm32.
2. **M1, first milestone: F1 + timeline + F2**
   - Import local files, and on desktop YouTube via `yt-dlp`.
   - HTDemucs separation through `ort` (desktop) and onnxruntime-web (web), with the stem cache.
   - Tempo, key and RMVPE reference-vocal analysis, plus reference note segmentation.
   - Svelte multitrack timeline: waveforms, mute, solo, gain, pan, transport, and a hidden reference-vocal MIDI track.
   - Record or import a take, then alignment, offline scoring and the score graph from §5.
   - Project save and load: files on desktop, OPFS plus archive export on the web.
   - **Done since:** RMVPE pitch on desktop and CLI (`Analysis.pitch_tracker` keeps reference and take on one tracker; web stays on YIN, since the model is 361 MB). Banded DTW after the coarse offset, with timing deviation reported apart from pitch. A CI workflow file.
   - **Done since (recording):**
     - In-app takes on desktop (cpal input → `rtrb` ring → writer thread) and web (raw `getUserMedia` into the player worklet's input → `SharedArrayBuffer` ring), with the backing playing. The UI's Recording state machine (`packages/ui/src/takes.ts`): Idle → CountIn (one bar at the song's tempo, clicked by the player) → Recording → Stopping → Scored | Failed. Live input meter, Breath while listening, headphones hint.
     - Alignment: the player reports when the count-in starts (desktop: cpal output timestamp vs the first input timestamp; web: the same render quantum), plus the count-in length and the stored latency. Takes live on the song timeline (`engine-core::take`), so scoring skips the coarse offset and keeps DTW. Takes are saved with the project (desktop: `take-<id>.wav` in the cache entry; web: in the `.grain` file) and re-scored from their stored pitch; imported take files become takes too.
     - Backing choice: Instrumental / Instrumental + guide vocal / Full mix, defaulting from `Analysis.accompaniment` (non-vocal stems vs mix, measured once at import or load; below −30 dB means a cappella → guide vocal, with the reason shown).
     - Loopback latency calibration (`engine-core::cue`: exponential chirp, FFT cross-correlation, sub-sample peak): stored per input/output device pair (desktop `latency.json` in app data; web `localStorage`), ± nudge by hand. Synthetic delayed chirps are recovered within 1 ms (0–287 ms, with noise and an echo).
     - A/B audition: clicking a note in the Score graph solos the reference vocal for that note, then the take (Audition state machine), and restores the mix.
     - Web persistence: OPFS behind an explicit Available/Unavailable state, Recent list, `.grain` export and import (§12).
   - **Done since (sidecars):** yt-dlp, ffmpeg and ffprobe bundled from pinned GitHub releases, lookup order and "Update extractor" (§4.1).
   - **Still open in M1:**
     - Deno (yt-dlp's JavaScript runtime for YouTube) is not bundled; see §4.1.
     - Take pre-processing from §5.1 (high-pass, loudness, bleed isolation) is not applied; the UI asks for headphones instead.
     - Calibration can only be checked by ear on real hardware: the automated tests use synthetic captures, and a headless browser cannot loop its output back into its input.
     - The RoFormer cascade is slow on CPU (~7 min for a 4 min song, about 8 GB peak), so it is an opt-in "Best vocals" import: a quality option for songs with real accompaniment. CoreML and int8 did not help (see §4.2).
     - Test-song note: `samples/eill-finale.mp3` ("Finale" by eill) is a cappella, a solo vocal with doubling and reverb and no backing music. Both HTDemucs and the cascade put nearly everything into Vocals, which is correct for this song; it is not a model limit.
3. **M2, F3:** LRCLIB lyrics, word alignment, live pitch lane, latency calibration. **Done** (see §6.5).
   - **Still open in M2:**
     - ML forced alignment of words (§6.1.2). Word timing is spread over the reference notes instead, so the wipe follows the melody, not the consonants.
     - SwiftF0 as a robust live estimator. Live is YIN only, which is fine with headphones and weaker with bleed.
     - Our own AEC for singing without headphones. The web host already asks for `echoCancellation`.
     - The live preview against an RMVPE reference compares YIN with RMVPE, so it can differ slightly from the final (RMVPE-vs-RMVPE) take score. The final score in Score is authoritative.
     - Lyrics for a song whose file is a different edit from LRCLIB's need the offset nudge or line edits. For example, "Finale" by eill: the file is 225 s, while LRCLIB's synced versions are 242–244 s and `/api/get` finds nothing, so the search fallback picks the nearest one.
     - Karaoke's live score is a preview: frames only, no timing or stability. Each run's full report is in Score.
4. **M3, F4:** live PSOLA auto-tune, then the offline high-quality shifter. **Done** (see §7.2).
   - **Still open in M3:**
     - ~~The editable correction curve~~: done in M4 as Auto-tune's "Edit curve…", which opens the correction in the pitch editor (§7.1.1).
     - The live monitor and the accuracy tests have been checked on synthetic voices and the benchmarks only. Real-mic feel (the 31.5 ms plus device latency) needs a listen on hardware.
     - The live path follows YIN, so it inherits YIN's octave slips on breathy notes. Offline tuned takes use the take's stored pitch (RMVPE on desktop).
     - Desktop monitoring needs the microphone to run at the output's rate; otherwise it reports Failed instead of resampling live.
     - Custom notes start from all twelve; they don't start from the current key.
     - Projects saved before M3 open with chromatic auto-tune settings.
5. **M4, quality + pitch editor:** the Melodyne-style pitch editor (§7.1) in a new **Pitch** session view, plus basic-pitch MIDI layers and a model manager with opt-in NC downloads. The RoFormer vocal cascade was split out as M4a and moved up. **Done** (see §7.1.1, §4.3, §10).
   - **Checked:** engine tests (edit curves, render accuracy, WSOLA, drums, SMF, model registry and state machine, checksum failure, a real download from GitHub, basic-pitch F1), UI state tests, and the web build end to end in headless Chrome. That run imported a synthetic song, transcribed the vocal stem, downloaded basic-pitch from GitHub into OPFS (with the dev copy blocked), and in the Pitch view did Correct pitch, ⌘Z, a mouse drag to the next scale note, and Render.
   - **Still open in M4:**
     - **MuScriptor** is registered (CC-BY-NC-4.0, opt-in) but can't be fetched: the Hugging Face repo is gated (anonymous downloads get 401) and ships PyTorch safetensors only. Wiring it up needs an ONNX export and a way to pass the user's Hugging Face token. The licence-acceptance dialog is in place for any downloadable opt-in model.
     - The Mel-Band RoFormer stays a local build (`[model.build]`): the panel shows the export command instead of a download.
     - The pitch editor uses the take's stored pitch track (RMVPE on desktop, YIN on web). There are no amplitude, sibilant or polyphonic tools, and no per-note transition *handles* on the canvas: glides are set with the Glide-in slider.
     - Moving a note in time re-times its audio (WSOLA), but the scored pitch of the new take is re-tracked, not mapped, so a moved note scores where it now sits.
     - Drum transcription is the simple classifier; toms, cymbals and flams need a model.
     - basic-pitch on web runs on the WASM EP (the model is tiny). Native runs on ORT CPU; there's no CoreML (§4.2).
     - Real-hardware listening checks (the edited takes' sound, and dragging feel on a trackpad) are still to do; the tests are synthetic.
6. **M5, F5:** DeepFilterNet and the vocal chain, then the ACE-Step local sidecar. **Done** (see §8.1).
   - **Checked:** engine tests (every chain stage, LUFS, Auto on synthetic bright/dark/quiet/loud
     voices, the live chain in the player, harmony intervals including C major E → G, the generator
     state machine, WAV codec), the DeepFilterNet SNR test (+12.7 dB), the sidecar's pytest
     contract tests and the Rust lifecycle test against the stub, UI state tests, and the web build
     in headless Chrome (Produce view: harmony and chain applied to an imported take).
   - **Still open in M5:**
     - ACE-Step has never generated in this repository: weights were not downloaded. The first
       real install and run (time, memory, MPS stability, the `acestep` Python API as pinned) is
       untested; the stub covers everything up to the model call.
     - `uv` isn't bundled. Linux/Windows installs resolve ACE-Step's CUDA torch wheels from
       download.pytorch.org (pinned in `uv.lock`), which is outside the model-host allow-list.
     - Generated stereo is folded to mono (takes are mono); generation length is capped at 600 s
       by ACE-Step.
     - Clean-up is desktop only and offline only (see §8.1); de-reverb is deferred.
     - The vocal chain's presets are tuned on synthetic voices; they need listening on real takes.
       Live chain meters aren't shown (only the offline before/after).
     - Harmony follows the detected key only (no chord awareness), and the score/auto-tune take
       pickers also list harmony and generated tracks.

### Everything still open (project-wide)

- **Import/separation:** Deno for yt-dlp isn't bundled; Best vocals is slow on CPU (~7 min);
  CoreML stays off (NaN); take pre-processing (§5.1) isn't applied.
- **Scoring/karaoke:** ML forced alignment of words; SwiftF0 live; our own AEC; the live preview
  compares YIN with RMVPE on desktop.
- **Auto-tune/pitch editor:** real-hardware listening and latency checks; no amplitude, sibilant or
  polyphonic tools; moved notes are re-tracked, not mapped; desktop monitoring needs matching
  input/output rates.
- **Models:** MuScriptor (gated, PyTorch-only) can't be fetched; the RoFormer is a local build;
  drum transcription is a simple classifier.
- **Produce:** the M5 list above (ACE-Step untested with real weights, `uv` not bundled, no web or
  live denoise, de-reverb deferred).
- **Design/licensing:** the Geist Pixel font's licence is unverified (README); the UI design pass
  row stays "in progress"; minimum web hardware (§12) is undecided.

---

## 12. Open decisions

- [x] Project archive format (web): one `.grain` file per project, which is also the OPFS record. 8-byte magic `GRAINv1\n`, u32 LE header length, a JSON header `{ "project": Project }`, then 16-bit little-endian PCM channels of `project.frames` samples in project order (each track's left then right channel, then each take). No dependencies, readable one channel at a time; FLAC compression can come later as `GRAINv2`. Desktop keeps the stem cache (WAV + `project.json`).
- [ ] Minimum hardware for the web build (WebGPU required, or a WASM-only slow path)?
