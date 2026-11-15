// Engine API shared by the desktop (Tauri) and web (WASM) hosts.
// Data types are generated from Rust by ts-rs (`pnpm types`); only the host contract is written here.

import type { ImportStage } from "./generated/ImportStage";
import type { Latency } from "./generated/Latency";
import type { LiveUpdate } from "./generated/LiveUpdate";
import type { LyricsUpdate } from "./generated/LyricsUpdate";
import type { MonitorMode } from "./generated/MonitorMode";
import type { MonitorStatus } from "./generated/MonitorStatus";
import type { ProjectLyrics } from "./generated/ProjectLyrics";
import type { PlayerCommand } from "./generated/PlayerCommand";
import type { Project } from "./generated/Project";
import type { ProjectSummary } from "./generated/ProjectSummary";
import type { ScoreReport } from "./generated/ScoreReport";
import type { ScoreSettings } from "./generated/ScoreSettings";
import type { SeparationQuality } from "./generated/SeparationQuality";
import type { TakeAdded } from "./generated/TakeAdded";
import type { TransportSnapshot } from "./generated/TransportSnapshot";
import type { TunePreview } from "./generated/TunePreview";
import type { TuneSettings } from "./generated/TuneSettings";
import type { EditOp } from "./generated/EditOp";
import type { EditSource } from "./generated/EditSource";
import type { EditUpdate } from "./generated/EditUpdate";
import type { PitchEdit } from "./generated/PitchEdit";
import type { PitchEditView } from "./generated/PitchEditView";
import type { ModelEntry } from "./generated/ModelEntry";
import type { ModelState } from "./generated/ModelState";
import type { TrackSource } from "./generated/TrackSource";
import type { TranscribeOutcome } from "./generated/TranscribeOutcome";
import type { TakeChain } from "./generated/TakeChain";
import type { ExportFormat } from "./generated/ExportFormat";
import type { ExportRequest } from "./generated/ExportRequest";
import type { ChainPick } from "./generated/ChainPick";
import type { ChainPreset } from "./generated/ChainPreset";
import type { ChainReport } from "./generated/ChainReport";
import type { ChainSettings } from "./generated/ChainSettings";
import type { GenerationRequest } from "./generated/GenerationRequest";
import type { GenerationTask } from "./generated/GenerationTask";
import type { GeneratorEvent } from "./generated/GeneratorEvent";
import type { GeneratorState } from "./generated/GeneratorState";
import type { Instrument } from "./generated/Instrument";
import type { Interval } from "./generated/Interval";
import type { Measures } from "./generated/Measures";

export type { Accompaniment } from "./generated/Accompaniment";
export type { Analysis } from "./generated/Analysis";
export type { Difficulty } from "./generated/Difficulty";
export type { FrameComparison } from "./generated/FrameComparison";
export type { FrameTotals } from "./generated/FrameTotals";
export type { LineEdit } from "./generated/LineEdit";
export type { LiveFrame } from "./generated/LiveFrame";
export type { LiveUpdate } from "./generated/LiveUpdate";
export type { LyricLine } from "./generated/LyricLine";
export type { LyricsQuery } from "./generated/LyricsQuery";
export type { LyricsUpdate } from "./generated/LyricsUpdate";
export type { ProjectLyrics } from "./generated/ProjectLyrics";
export type { Syllable } from "./generated/Syllable";
export type { ImportStage } from "./generated/ImportStage";
export type { SeparationQuality } from "./generated/SeparationQuality";
export type { Key } from "./generated/Key";
export type { KeyCandidate } from "./generated/KeyCandidate";
export type { Latency } from "./generated/Latency";
export type { LatencySource } from "./generated/LatencySource";
export type { MixParams } from "./generated/MixParams";
export type { Mode } from "./generated/Mode";
export type { MonitorMode } from "./generated/MonitorMode";
export type { MonitorStatus } from "./generated/MonitorStatus";
export type { Note } from "./generated/Note";
export type { NoteReport } from "./generated/NoteReport";
export type { NoteResult } from "./generated/NoteResult";
export type { OctaveMode } from "./generated/OctaveMode";
export type { PitchFrame } from "./generated/PitchFrame";
export type { PitchTrack } from "./generated/PitchTrack";
export type { PlayerCommand } from "./generated/PlayerCommand";
export type { Project } from "./generated/Project";
export type { ProjectSummary } from "./generated/ProjectSummary";
export type { ReferenceSource } from "./generated/ReferenceSource";
export type { ScoreReport } from "./generated/ScoreReport";
export type { Scores } from "./generated/Scores";
export type { ScoreSettings } from "./generated/ScoreSettings";
export type { SongSource } from "./generated/SongSource";
export type { Take } from "./generated/Take";
export type { TakeAdded } from "./generated/TakeAdded";
export type { TakeSource } from "./generated/TakeSource";
export type { Stem } from "./generated/Stem";
export type { Tempo } from "./generated/Tempo";
export type { Track } from "./generated/Track";
export type { TrackMode } from "./generated/TrackMode";
export type { TrackSource } from "./generated/TrackSource";
export type { Transport } from "./generated/Transport";
export type { TransportSnapshot } from "./generated/TransportSnapshot";
export type { Scale } from "./generated/Scale";
export type { TuneMode } from "./generated/TuneMode";
export type { TunePreview } from "./generated/TunePreview";
export type { TuneSettings } from "./generated/TuneSettings";
export type { Vibrato } from "./generated/Vibrato";
export type { EditNote } from "./generated/EditNote";
export type { EditOp } from "./generated/EditOp";
export type { EditSource } from "./generated/EditSource";
export type { EditUpdate } from "./generated/EditUpdate";
export type { PitchEdit } from "./generated/PitchEdit";
export type { PitchEditView } from "./generated/PitchEditView";
export type { Snap } from "./generated/Snap";
export type { Distribution } from "./generated/Distribution";
export type { ModelEntry } from "./generated/ModelEntry";
export type { ModelEvent } from "./generated/ModelEvent";
export type { ModelFile } from "./generated/ModelFile";
export type { ModelInfo } from "./generated/ModelInfo";
export type { ModelState } from "./generated/ModelState";
export type { Obtain } from "./generated/Obtain";
export type { MidiNote } from "./generated/MidiNote";
export type { Transcription } from "./generated/Transcription";
export type { TranscriptionMethod } from "./generated/TranscriptionMethod";
export type { TranscribeOutcome } from "./generated/TranscribeOutcome";
export type { ChainPick } from "./generated/ChainPick";
export type { ChainPreset } from "./generated/ChainPreset";
export type { ChainReport } from "./generated/ChainReport";
export type { ChainSettings } from "./generated/ChainSettings";
export type { TakeChain } from "./generated/TakeChain";
export type { Step } from "./generated/Step";
export type { PitchStep } from "./generated/PitchStep";
export type { CleanStep } from "./generated/CleanStep";
export type { VocalStep } from "./generated/VocalStep";
export type { ExportFormat } from "./generated/ExportFormat";
export type { ExportRange } from "./generated/ExportRange";
export type { ExportRequest } from "./generated/ExportRequest";
export type { GenerationRequest } from "./generated/GenerationRequest";
export type { GenerationTask } from "./generated/GenerationTask";
export type { GeneratorEvent } from "./generated/GeneratorEvent";
export type { GeneratorState } from "./generated/GeneratorState";
export type { Instrument } from "./generated/Instrument";
export type { Interval } from "./generated/Interval";
export type { Measures } from "./generated/Measures";

export type ImportOutcome = { kind: "imported"; project: Project } | { kind: "cancelled" };

export type ScoreOutcome = { kind: "scored"; added: TakeAdded } | { kind: "cancelled" };

export type StageListener = (stage: ImportStage) => void;

/** Where the yt-dlp in use came from (AGENTS.md §4.1): an updated copy in app data, the
 * sidecar bundled with the app, or the system's (e.g. Homebrew). */
export type ExtractorOrigin = "updated" | "bundled" | "system";

export type ExtractorUpdate = { kind: "updated" | "upToDate"; version: string; origin: ExtractorOrigin };

/** The YouTube extractor (yt-dlp). `version()` is slow-ish, so the UI asks for it lazily. */
export type Extractor = {
  origin: ExtractorOrigin;
  version(): Promise<string>;
  /** Updates yt-dlp from its GitHub releases; a system yt-dlp is left to its package manager. */
  update(): Promise<ExtractorUpdate>;
};

/** YouTube import exists only on desktop (AGENTS.md §0). `missing`: the build supports it but
 * no yt-dlp was found. */
export type UrlImport =
  | {
      kind: "available";
      extractor: Extractor;
      importUrl(url: string, quality: SeparationQuality, onStage: StageListener): Promise<ImportOutcome>;
    }
  | { kind: "missing"; reason: string }
  | { kind: "unavailable" };

/** Which separations the UI may offer (AGENTS.md §4.2): the choice of `Fast` or `BestVocals`,
 * or `Fast` only, with the reason shown next to the disabled option. */
export type SeparationOffer = { kind: "choice" } | { kind: "fastOnly"; reason: string };

/** How the UI meets the window frame. `overlay`: the OS window buttons sit over the top-left
 * of the page (macOS desktop), so the top bar insets its content and acts as the drag area:
 * pressing any non-interactive point of it starts a window drag, double-clicking zooms. */
export type WindowChrome =
  | { kind: "overlay"; insetLeft: number; startDrag(): void; toggleMaximize(): void }
  | { kind: "plain" };

/** One-file `.grain` project archives (AGENTS.md §12): export the loaded song, or open one. */
export type ProjectArchive =
  | {
      kind: "available";
      /** Downloads the loaded song as a `.grain` file. */
      exportSong(): Promise<void>;
      /** Picks a `.grain` file, stores it and makes it the loaded song. */
      importArchive(): Promise<ImportOutcome>;
    }
  | { kind: "unavailable" };

/** Saved projects (desktop: the stem cache; web: OPFS). */
export type ProjectStore =
  | {
      kind: "available";
      list(): Promise<ProjectSummary[]>;
      /** Opens a saved project and makes it the loaded song. */
      open(id: string): Promise<Project>;
      /** Saves the loaded song: mix settings and takes. */
      save(): Promise<void>;
      archive: ProjectArchive;
    }
  | { kind: "unavailable"; reason: string };

/** The microphone, for recording takes over the song and measuring round-trip latency
 * (AGENTS.md §9). The UI owns the recording and calibration state machines; hosts give it
 * these primitives. */
export type Microphone =
  | {
      kind: "available";
      /** Opens the microphone and starts the one-bar count-in from the current position. The
       * song plays on after it; the transport shows `CountIn`, then `Playing`. */
      start(): Promise<void>;
      /** Peak input level of the latest block, 0–1. */
      level(): Promise<number>;
      /** Live karaoke pitch while recording (AGENTS.md §6.3): frames sung since the previous
       * call, on the song timeline with the round-trip latency removed, each compared with the
       * reference by §5's per-frame rule, plus the run's running totals. Empty until the
       * count-in has started. */
      live(settings: ScoreSettings): Promise<LiveUpdate>;
      /** Stops, removes the round-trip latency, adds the take to the song and scores it. */
      finish(settings: ScoreSettings): Promise<TakeAdded>;
      /** Abandons the take. */
      cancel(): Promise<void>;
      /** Latency removed from takes on the current device pair. */
      latency(): Promise<Latency>;
      setLatency(ms: number): Promise<Latency>;
      /** Plays the loopback chirp and stores the measured latency. */
      measureLatency(): Promise<Latency>;
    }
  | { kind: "unavailable"; reason: string };

/** Auto-tune (AGENTS.md §7): live monitoring through the player and the preview. Applying it is
 * a step of the take's processing chain (`TakeStore.setChain`, AGENTS.md §8.7). */
export type AutoTune = {
  /** Turns live monitoring off, or on dry or tuned. Rejects (with the reason) when the
   * microphone can't be opened. */
  setMonitor(mode: MonitorMode): Promise<MonitorStatus>;
  /** The live tuner's settings, used from the next audio block. */
  setTune(settings: TuneSettings): Promise<void>;
  /** What tuning take `take` would do (from its stored pitch, instant). */
  preview(take: number, settings: TuneSettings): Promise<TunePreview>;
};

/** The pitch editor (AGENTS.md §7.1): the edit list of a take's Pitch edits step. Every change
 * is kept with the loaded project (saved on the next Save); the session history keeps undo/redo
 * as snapshots of the edit list. Hearing the edits is the chain step (`TakeStore.setChain`). */
export type PitchEditor = {
  /** A take or the vocal stem: its saved edit list (or a fresh analysis), pitch and loudness. */
  open(source: EditSource): Promise<PitchEditView>;
  /** Applies one edit to `edit` and keeps the result. */
  apply(edit: PitchEdit, op: EditOp): Promise<EditUpdate>;
  /** Replaces a source's edit list (undo and redo). */
  set(edit: PitchEdit): Promise<EditUpdate>;
  /** Seeds take `take`'s edit list from auto-tune's settings (the editable correction curve). */
  fromTune(take: number, settings: TuneSettings): Promise<EditUpdate>;
};

/** The model manager (AGENTS.md §10): every manifest model, downloaded on request into the
 * host's model store (app data on desktop, the origin's private storage on the web) and verified
 * against its checksum. */
export type ModelManager = {
  /** Where downloads are kept, for display. */
  readonly location: string;
  list(): Promise<ModelEntry[]>;
  /** Downloads and verifies model `id`. Opt-in (restricted) models need `acceptLicense`, which
   * the UI only passes after the user accepted the licence. `onState` sees each state change;
   * resolves with the final state (`Ready` or `Failed`). */
  download(id: string, acceptLicense: boolean, onState: (state: ModelState) => void): Promise<ModelState>;
  /** Deletes a downloaded model. */
  delete(id: string): Promise<ModelState>;
};

/** Saving a file the user picks a place for. */
export type ExportOutcome = { kind: "saved" } | { kind: "cancelled" };

/** MIDI layers (AGENTS.md §4.3): basic-pitch for pitched tracks, a classifier for drums. */
export type Transcriber = {
  /** Transcribes a track of the loaded song and keeps the layer with the project. */
  transcribe(source: TrackSource): Promise<TranscribeOutcome>;
  /** Saves a track's MIDI layer as a .mid file. */
  exportMidi(source: TrackSource): Promise<ExportOutcome>;
};

/** Whether the Clean up chain step can run (DeepFilterNet: tract on the CPU on desktop; the web
 * build has no runtime for it). */
export type Denoiser = { kind: "available"; model: string } | { kind: "unavailable"; reason: string };

/** ACE-Step generation (AGENTS.md §8). `local`: the opt-in sidecar on this machine, installed
 * from the Models panel; `remote`: a backend URL configured for this build (nothing to
 * install). */
export type Generator =
  | {
      kind: "available";
      where: "local" | "remote";
      /** Honest expectations: download size, memory, time per generation on this machine. */
      expectations: string;
      state(): Promise<GeneratorState>;
      /** Installs the sidecar (local only); `onState` sees each step. */
      install(onState: (s: GeneratorState) => void): Promise<GeneratorState>;
      uninstall(): Promise<GeneratorState>;
      /** Runs one generation and adds it as a new track; `onState` sees Running → Ready. */
      generate(task: GenerationTask, prompt: string, seed: number, settings: ScoreSettings, onState: (s: GeneratorState) => void): Promise<TakeAdded>;
    }
  | { kind: "unavailable"; reason: string };

/** Produce (AGENTS.md §8). Clean up and the vocal chain are steps of the take's processing chain
 * (`TakeStore.setChain`); harmony and generation make new tracks. The live vocal chain is a
 * player command (`{ kind: "Chain" }` through `send`). */
export type Producer = {
  /** Measures a source and gives `preset`'s settings for it. */
  chainPick(source: EditSource, preset: ChainPreset): Promise<ChainPick>;
  /** Sings a diatonic harmony from the source's own voice as a new track (audible, panned). */
  harmony(source: EditSource, interval: Interval, settings: ScoreSettings): Promise<TakeAdded>;
  /** Saves the source's melody, or its harmony at `interval`, as a .mid file. */
  exportMelody(source: EditSource, interval: Interval | null): Promise<ExportOutcome>;
  readonly denoise: Denoiser;
  readonly generator: Generator;
};

/** Whether no step of a chain changes the take (mirrors engine-core `TakeChain::is_neutral`). */
export function chainIsNeutral(c: TakeChain): boolean {
  return [c.pitch, c.tune, c.clean, c.vocal].every((s) => s.state !== "Active");
}

/** A `.mid` name for a source's melody (or harmony) export. */
export function melodyFileName(source: EditSource, interval: Interval | null): string {
  const base = source.kind === "VocalStem" ? "Melody" : `Take ${source.id}`;
  return interval ? `${base} (${interval}).mid` : `${base}.mid`;
}

/** What a chain change did: rendered (the player now plays the result), or the Clean up step
 * needs its model downloaded first (nothing changed). Mirrors the desktop host's `ChainOutcome`. */
export type ChainOutcome = { kind: "Rendered"; project: Project } | { kind: "NeedsModel"; model: string };

export type TakeStore = {
  rename(id: number, name: string): Promise<Project>;
  /** Deletes take `id` and its audio. The player is rebuilt without it, so playback stops. */
  remove(id: number): Promise<Project>;
  /** Replaces take `id`'s processing chain (AGENTS.md §8.7) and renders it off the main thread,
   * re-running only from the first step that changed; saved at once. One call at a time. */
  setChain(id: number, chain: TakeChain): Promise<ChainOutcome>;
  /** Stars take `id` as the master (`null`: the newest take is); saved at once. */
  setMaster(id: number | null): Promise<Project>;
};

/** One export format and whether this host can write it. */
export type FormatOffer = { format: ExportFormat; availability: { kind: "ready" } | { kind: "unavailable"; reason: string } };

/** Pushed while an export runs (the dialog's Rendering and Encoding states). */
export type ExportProgress = { kind: "Rendering"; fraction: number } | { kind: "Encoding" };

export type MixdownOutcome = { kind: "saved"; path: string } | { kind: "cancelled" };

/** Export (AGENTS.md §8.8): the mixdown of the chosen tracks at their mix levels, takes through
 * their chains. The host picks the destination (save dialog or download). */
export type Exporter = {
  formats(): Promise<FormatOffer[]>;
  run(request: ExportRequest, name: string, onProgress: (p: ExportProgress) => void): Promise<MixdownOutcome>;
};

export interface EngineHost {
  readonly chrome: WindowChrome;
  readonly urlImport: UrlImport;
  readonly separation: SeparationOffer;
  readonly projects: ProjectStore;
  /** Stops playback and unloads the song. */
  close(): Promise<void>;
  /** Picks a song file, imports it and makes it the loaded song. */
  importFile(quality: SeparationQuality, onStage: StageListener): Promise<ImportOutcome>;
  readonly microphone: Microphone;
  readonly autotune: AutoTune;
  readonly pitchEditor: PitchEditor;
  readonly models: ModelManager;
  readonly transcriber: Transcriber;
  readonly producer: Producer;
  /** Picks a sung take file, adds it to the loaded song and scores it. */
  scoreTake(settings: ScoreSettings): Promise<ScoreOutcome>;
  /** Sets the loaded song's lyrics (fetched by the UI from LRCLIB, or edited) and returns them
   * timed against the reference notes. Kept with the project on the next save. */
  setLyrics(update: LyricsUpdate): Promise<ProjectLyrics>;
  /** Re-scores take `id` of the loaded song (difficulty or octave changed, or picked again). */
  scoreRecorded(id: number, settings: ScoreSettings): Promise<ScoreReport>;
  /** Renaming and deleting takes. Every take change is saved at once, as is every call that
   * makes a take (recording, import, auto-tune, pitch render, Produce): takes autosave
   * (AGENTS.md §11). Saving writes the whole song, so it also keeps unsaved mix changes. */
  readonly takes: TakeStore;
  readonly exporter: Exporter;
  /** Sends a player command. Fire-and-forget: the UI never waits on it to draw. */
  send(command: PlayerCommand): Promise<void>;
  /** The transport now (one-off reads). */
  transport(): Promise<TransportSnapshot>;
  /** Pushes the transport to `listener` while the playhead moves (~40 Hz) and on every other
   * change; the UI extrapolates between pushes (AGENTS.md §10.1). Returns the unsubscribe. */
  subscribeTransport(listener: (s: TransportSnapshot) => void): () => void;
}
