// The loaded song's UI state (AGENTS.md ground rules 1–2, §10.1).
//
// `project` and `score` hold engine data that runs to megabytes (waveform peaks, pitch tracks,
// analysis, score frames). They are raw state: replaced whole, never deep-proxied, so reading
// them costs nothing and a change is one signal, not thousands. Everything else is small.

import type { EditSource, Project, ScoreSettings } from "@workstation/engine-api";
import type { SaveState, ScoreState, ViewId } from "./state";
import type { Backing } from "./takes";

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
  /** What the Pitch view edits (`null`: its default, the newest take). */
  pitchSource = $state<EditSource | null>(null);

  constructor(project: Project) {
    this.project = project;
  }

  /** Replaces the project and marks the song unsaved. */
  adopt(project: Project): void {
    this.project = project;
    this.touch();
  }

  /** Marks the song as having unsaved changes. */
  touch(): void {
    if (this.save.kind === "clean") this.save = { kind: "dirty" };
  }
}
