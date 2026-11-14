// Engine API shared by the desktop (Tauri) and web (WASM) hosts.
// Data types are generated from Rust by ts-rs (`pnpm types`); only the host contract is written here.

import type { ImportStage } from "./generated/ImportStage";
import type { PlayerCommand } from "./generated/PlayerCommand";
import type { Project } from "./generated/Project";
import type { ScoreReport } from "./generated/ScoreReport";
import type { ScoreSettings } from "./generated/ScoreSettings";
import type { TransportSnapshot } from "./generated/TransportSnapshot";

export type { Analysis } from "./generated/Analysis";
export type { Difficulty } from "./generated/Difficulty";
export type { FrameComparison } from "./generated/FrameComparison";
export type { ImportStage } from "./generated/ImportStage";
export type { Key } from "./generated/Key";
export type { KeyCandidate } from "./generated/KeyCandidate";
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
export type { ReferenceSource } from "./generated/ReferenceSource";
export type { ScoreReport } from "./generated/ScoreReport";
export type { Scores } from "./generated/Scores";
export type { ScoreSettings } from "./generated/ScoreSettings";
export type { SongSource } from "./generated/SongSource";
export type { Stem } from "./generated/Stem";
export type { Tempo } from "./generated/Tempo";
export type { Track } from "./generated/Track";
export type { TrackMode } from "./generated/TrackMode";
export type { TrackSource } from "./generated/TrackSource";
export type { Transport } from "./generated/Transport";
export type { TransportSnapshot } from "./generated/TransportSnapshot";
export type { Vibrato } from "./generated/Vibrato";

export type ImportOutcome = { kind: "imported"; project: Project } | { kind: "cancelled" };

export type ScoreOutcome = { kind: "scored"; report: ScoreReport } | { kind: "cancelled" };

export type StageListener = (stage: ImportStage) => void;

/** YouTube import exists only on desktop (AGENTS.md §0). */
export type UrlImport =
  | { kind: "available"; importUrl(url: string, onStage: StageListener): Promise<ImportOutcome> }
  | { kind: "unavailable" };

/** Everything the UI may ask of an engine host. Each host shows its own file pickers. */
export interface EngineHost {
  readonly urlImport: UrlImport;
  /** Picks a song file, imports it and makes it the loaded song. */
  importFile(onStage: StageListener): Promise<ImportOutcome>;
  /** Picks a sung take and scores it against the loaded song. */
  scoreTake(settings: ScoreSettings): Promise<ScoreOutcome>;
  send(command: PlayerCommand): Promise<void>;
  transport(): Promise<TransportSnapshot>;
}
