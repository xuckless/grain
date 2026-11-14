// Engine API shared by the desktop (Tauri) and web (WASM) hosts.
// Data types are generated from Rust by ts-rs (`pnpm types`); only the host contract is written here.

import type { ImportStage } from "./generated/ImportStage";
import type { Latency } from "./generated/Latency";
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

/** YouTube import exists only on desktop (AGENTS.md §0). */
export type UrlImport =
  | { kind: "available"; importUrl(url: string, quality: SeparationQuality, onStage: StageListener): Promise<ImportOutcome> }
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

/** Saved projects (desktop: the stem cache). */
export type ProjectStore =
  | {
      kind: "available";
      list(): Promise<ProjectSummary[]>;
      /** Opens a saved project and makes it the loaded song. */
      open(id: string): Promise<Project>;
      /** Saves the loaded song's mix settings. */
      save(): Promise<void>;
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
  /** Re-scores take `id` of the loaded song (difficulty or octave changed, or picked again). */
  scoreRecorded(id: number, settings: ScoreSettings): Promise<ScoreReport>;
  send(command: PlayerCommand): Promise<void>;
  transport(): Promise<TransportSnapshot>;
}
