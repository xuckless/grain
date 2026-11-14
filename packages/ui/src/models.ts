// Model manager and MIDI-layer helpers (AGENTS.md §10, §4.3): the UI's explicit states and the
// pure functions behind the Models panel and the Mix view's MIDI lanes.

import type { MidiNote, ModelEntry, ModelInfo, ModelState, Project, TrackSource, Transcription } from "@workstation/engine-api";

// ---- Model manager ----------------------------------------------------------------------------

/** Asking the user to accept a restricted model's licence before downloading it. */
export type Consent = { kind: "none" } | { kind: "asking"; info: ModelInfo };

/** What the panel offers for a model in a state. */
export type ModelAction = "download" | "retry" | "delete" | "installed" | "busy" | "manual" | "build";

export function modelAction(entry: ModelEntry): ModelAction {
  const s = entry.state;
  if (s.kind === "Downloading" || s.kind === "Verifying") return "busy";
  if (s.kind === "Ready") return entry.removable ? "delete" : "installed";
  switch (entry.info.obtain.kind) {
    case "Manual":
      return "manual";
    case "Build":
      return "build";
    case "Download":
      return s.kind === "Failed" ? "retry" : "download";
  }
}

/** Download progress 0–1, or null when not downloading. */
export function progress(s: ModelState): number | null {
  if (s.kind !== "Downloading") return null;
  return s.total > 0 ? Math.min(1, s.received / s.total) : 0;
}

export function stateLabel(s: ModelState): string {
  switch (s.kind) {
    case "NotDownloaded":
      return "Not downloaded";
    case "Downloading":
      return `Downloading ${Math.round((progress(s) ?? 0) * 100)}%`;
    case "Verifying":
      return "Verifying";
    case "Ready":
      return "Ready";
    case "Failed":
      return "Failed";
  }
}

export function formatBytes(bytes: number | null): string {
  if (bytes === null) return "size unknown";
  if (bytes < 1e6) return `${Math.max(1, Math.round(bytes / 1e3))} kB`;
  if (bytes < 1e9) return `${Math.round(bytes / 1e6)} MB`;
  return `${(bytes / 1e9).toFixed(2)} GB`;
}

/** A model's total size, when every file's size is known. */
export function modelSize(info: ModelInfo): number | null {
  let total = 0;
  for (const f of info.files) {
    if (f.size === null) return null;
    total += f.size;
  }
  return total;
}

/** `entries` with model `id`'s state replaced (progress events). */
export function withState(entries: ModelEntry[], id: string, state: ModelState): ModelEntry[] {
  return entries.map((e) => (e.info.id === id ? { ...e, state } : e));
}

// ---- MIDI layers ------------------------------------------------------------------------------

/** Transcribing one track. `needsModel`: the manifest id of the model to download first. */
export type MidiJob = { kind: "idle" } | { kind: "transcribing" } | { kind: "needsModel"; model: string } | { kind: "failed"; message: string };

export function sameSource(a: TrackSource, b: TrackSource): boolean {
  return a.kind === b.kind && (a.kind === "Mix" || (b.kind === "Stem" && a.stem === b.stem));
}

export function layerOf(project: Project, source: TrackSource): Transcription | null {
  return project.transcriptions.find((t) => sameSource(t.source, source)) ?? null;
}

/** Vertical position (0 top – 1 bottom) of each note in a lane: pitches spread over the lane's
 * range; drum keys on fixed rows (hat top, snare middle, kick bottom). */
export function laneRows(t: Transcription): (n: MidiNote) => number {
  if (t.method === "DrumClassifier") {
    const rows: Record<number, number> = { 42: 0.2, 38: 0.5, 36: 0.8 };
    return (n) => rows[n.pitch] ?? 0.5;
  }
  if (t.notes.length === 0) return () => 0.5;
  const lo = Math.min(...t.notes.map((n) => n.pitch));
  const hi = Math.max(...t.notes.map((n) => n.pitch));
  return (n) => (hi === lo ? 0.5 : 0.85 - ((n.pitch - lo) / (hi - lo)) * 0.7);
}
