// Engine API shared by the desktop (Tauri) and web (WASM) hosts.
// Data types are generated from Rust by ts-rs (`pnpm types`); only the host contract is written here.

import type { ImportStage } from "./generated/ImportStage";
import type { Latency } from "./generated/Latency";
import type { LiveUpdate } from "./generated/LiveUpdate";
import type { LyricsUpdate } from "./generated/LyricsUpdate";
import type { ProjectLyrics } from "./generated/ProjectLyrics";
import type { PlayerCommand } from "./generated/PlayerCommand";
import type { Project } from "./generated/Project";
import type { ProjectSummary } from "./generated/ProjectSummary";
import type { ScoreReport } from "./generated/ScoreReport";
import type { ScoreSettings } from "./generated/ScoreSettings";
import type { SeparationQuality } from "./generated/SeparationQuality";
import type { TakeAdded } from "./generated/TakeAdded";
import type { TransportSnapshot } from "./generated/TransportSnapshot";

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
export type { Vibrato } from "./generated/Vibrato";

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

/** One-file project archives (web: export the loaded song, or open an exported one). */
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

/** Everything the UI may ask of an engine host. Each host shows its own file pickers. */
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
  /** Picks a sung take file, adds it to the loaded song and scores it. */
  scoreTake(settings: ScoreSettings): Promise<ScoreOutcome>;
  /** Sets the loaded song's lyrics (fetched by the UI from LRCLIB, or edited) and returns them
   * timed against the reference notes. Kept with the project on the next save. */
  setLyrics(update: LyricsUpdate): Promise<ProjectLyrics>;
  /** Re-scores take `id` of the loaded song (difficulty or octave changed, or picked again). */
  scoreRecorded(id: number, settings: ScoreSettings): Promise<ScoreReport>;
  send(command: PlayerCommand): Promise<void>;
  transport(): Promise<TransportSnapshot>;
}
