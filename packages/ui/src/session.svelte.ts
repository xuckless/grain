// The loaded song's UI state (AGENTS.md ground rules 1–2, §10.1).
//
// `project` and `score` hold engine data that runs to megabytes (waveform peaks, pitch tracks,
// analysis, score frames). They are raw state: replaced whole, never deep-proxied, so reading
// them costs nothing and a change is one signal, not thousands. Everything else is small.

import type { Project, ScoreSettings } from "@workstation/engine-api";
import type { SaveState, ScoreState, ViewId } from "./state";
import { DEFAULT_LISTEN_MODE, type Listening, type ListenMode } from "./listen";
import type { Backing } from "./takes";
import { emptyHistory, record, type Command, type History } from "./history";
import { fitView, type View } from "./viewport";
import type { MonitorState } from "./autotune";

/** Who has the microphone: nobody, a take being recorded, or a latency calibration. */
export type MicUse = { kind: "free" } | { kind: "recording" } | { kind: "calibrating" };

export class Session {
  /** Replace, never mutate: `session.project = { ...session.project, … }`. */
  project = $state.raw({} as Project);
  score = $state.raw<ScoreState>({ kind: "none" });
  save = $state<SaveState>({ kind: "clean" });
  view = $state<ViewId>("Mix");
  /** Difficulty and octave folding, shared by Score and Karaoke. */
  settings = $state<ScoreSettings>({ difficulty: "Medium", octave: "Fold" });
  /** What the singer hears while recording; `null` until they pick (the song's default applies). */
  backing = $state<Backing | null>(null);
  /** Bumped when an undo or redo changes a pitch edit list, so the Pitch view reloads it. */
  pitchRevision = $state(0);
  /** Hearing a take (one at a time, app-wide; driven by SessionScreen, see listen.ts). */
  listening = $state.raw<Listening>({ kind: "idle" });
  /** How takes play back (AGENTS.md §8.6); never with the guide vocal. */
  playback = $state<ListenMode>(DEFAULT_LISTEN_MODE);
  /** The take Score, Karaoke, Auto-tune, Pitch and Produce work on (`null`: the ★ master). */
  take = $state<number | null>(null);
  /** The shared timeline viewport (one zoom and scroll across views, viewport.ts). */
  viewport = $state.raw<View>({ start: 0, span: 1, overshoot: 0 });
  /** Live monitoring (hear yourself), shared by Settings and Auto-tune. */
  monitor = $state<MonitorState>({ kind: "off" });
  mic = $state<MicUse>({ kind: "free" });
  /** The Karaoke loop (A–B, song seconds), also an export range. */
  loop = $state.raw<{ start: number; end: number } | null>(null);
  /** ⌘Z / ⇧⌘Z (history.ts). */
  history = $state.raw<History>(emptyHistory);

  constructor(project: Project) {
    this.project = project;
    this.viewport = fitView(project.frames / project.sample_rate);
  }

  /** Song length in seconds. */
  get duration(): number {
    return this.project.frames / this.project.sample_rate;
  }

  /** Remembers a change for undo. */
  record(c: Command): void {
    this.history = record(this.history, c);
  }

  /** Replaces the project and marks the song unsaved. */
  adopt(project: Project): void {
    this.project = project;
    this.touch();
  }

  /** Replaces the project with one the host just saved (every take change autosaves, and a
   * save writes the whole song, AGENTS.md §11). */
  saved(project: Project): void {
    this.project = project;
    this.save = { kind: "clean" };
  }

  /** Marks the song as having unsaved changes. */
  touch(): void {
    if (this.save.kind === "clean") this.save = { kind: "dirty" };
  }
}
