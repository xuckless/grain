// The Produce view (AGENTS.md §8): its explicit states and the pure functions behind it. No
// Svelte or host imports, so they are unit-tested under Node (`pnpm --filter @workstation/ui test`).

import type {
  ChainPreset,
  ChainSettings,
  EditSource,
  GenerationTask,
  GeneratorState,
  Instrument,
  Interval,
  Measures,
  Project,
  Take,
} from "@workstation/engine-api";

// ---- Jobs ------------------------------------------------------------------------------------

/** One Produce action (clean up, chain, harmony, generate): idle → running → done | needsModel
 * | failed, and back to running on a retry. `take`: the take or track it added. */
export type Job =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "done"; take: number }
  | { kind: "needsModel"; model: string }
  | { kind: "failed"; message: string };

export type JobEvent = { kind: "start" } | { kind: "added"; take: number } | { kind: "needsModel"; model: string } | { kind: "fail"; message: string };

export function jobNext(j: Job, e: JobEvent): Job | null {
  if (e.kind === "start") return j.kind === "running" ? null : { kind: "running" };
  if (j.kind !== "running") return null;
  switch (e.kind) {
    case "added":
      return { kind: "done", take: e.take };
    case "needsModel":
      return { kind: "needsModel", model: e.model };
    case "fail":
      return { kind: "failed", message: e.message };
  }
}

// ---- Sources and outputs ---------------------------------------------------------------------

/** Take sources that Produce makes. */
const PRODUCED = new Set<Take["source"]["kind"]>(["Denoised", "Chained", "Harmony", "Generated"]);

/** Takes and tracks made in Produce, newest first, for the outputs list. */
export function producedTakes(project: Project): Take[] {
  return project.takes.filter((t) => PRODUCED.has(t.source.kind)).reverse();
}

/** What a produced take keeps about how it was made, for its row. */
export function describeOutput(t: Take): string {
  const s = t.source;
  switch (s.kind) {
    case "Denoised":
      return `${s.model}`;
    case "Chained":
      return `${presetLabel(s.preset)} chain`;
    case "Harmony":
      return `${intervalLabel(s.interval)}, same voice`;
    case "Generated":
      return `${s.request.model} · seed ${s.request.seed}${s.request.prompt ? ` · “${s.request.prompt}”` : ""}`;
    default:
      return "";
  }
}

// ---- Vocal chain -----------------------------------------------------------------------------

export const PRESETS: readonly ChainPreset[] = ["Auto", "Natural", "Pop", "Rnb", "Rock", "HipHop", "Ballad"];

export function presetLabel(p: ChainPreset): string {
  switch (p) {
    case "Rnb":
      return "R&B";
    case "HipHop":
      return "Hip-hop";
    default:
      return p;
  }
}

/** One chain control: a slider over `key` in `[min, max]`. */
export type ChainControl = { key: keyof ChainSettings; label: string; min: number; max: number; step: number; format: (v: number) => string };

const db = (v: number) => `${v > 0 ? "+" : v < 0 ? "−" : ""}${Math.abs(v).toFixed(1)} dB`;
const pct = (v: number) => `${Math.round(v * 100)}%`;

/** The chain's controls, in signal order. */
export const CHAIN_CONTROLS: readonly ChainControl[] = [
  { key: "highpass_hz", label: "High-pass", min: 0, max: 300, step: 5, format: (v) => (v <= 0 ? "Off" : `${Math.round(v)} Hz`) },
  { key: "deess_max_db", label: "De-ess", min: 0, max: 12, step: 0.5, format: (v) => (v <= 0 ? "Off" : `up to ${v.toFixed(1)} dB`) },
  { key: "comp_ratio", label: "Compress", min: 1, max: 8, step: 0.1, format: (v) => (v <= 1 ? "Off" : `${v.toFixed(1)} : 1`) },
  { key: "low_db", label: "Low", min: -12, max: 12, step: 0.5, format: db },
  { key: "presence_db", label: "Presence", min: -12, max: 12, step: 0.5, format: db },
  { key: "air_db", label: "Air", min: -12, max: 12, step: 0.5, format: db },
  { key: "drive", label: "Saturation", min: 0, max: 1, step: 0.01, format: pct },
  { key: "reverb_mix", label: "Reverb", min: 0, max: 1, step: 0.01, format: pct },
  { key: "delay_mix", label: "Delay", min: 0, max: 1, step: 0.01, format: pct },
  { key: "output_db", label: "Output", min: -24, max: 24, step: 0.5, format: db },
];

/** One meter row: a measure before and after, on a 0–1 scale for the dot bar. */
export type MeterRow = { label: string; before: string; after: string; beforeFill: number; afterFill: number };

const fill = (v: number, lo: number, hi: number) => Math.max(0, Math.min(1, (v - lo) / (hi - lo)));

export function meterRows(before: Measures, after: Measures | null): MeterRow[] {
  const row = (label: string, get: (m: Measures) => number, unit: string, lo: number, hi: number): MeterRow => ({
    label,
    before: `${get(before).toFixed(1)} ${unit}`,
    after: after ? `${get(after).toFixed(1)} ${unit}` : "",
    beforeFill: fill(get(before), lo, hi),
    afterFill: after ? fill(get(after), lo, hi) : 0,
  });
  return [
    row("Loudness", (m) => m.lufs, "LUFS", -50, 0),
    row("Peak", (m) => m.peak_db, "dBFS", -40, 0),
    row("Brightness", (m) => m.tilt_db, "dB", -35, 5),
    row("Sibilance", (m) => m.sibilance_db, "dB", -45, 0),
  ];
}

// ---- Harmony ---------------------------------------------------------------------------------

export const INTERVALS: readonly Interval[] = ["ThirdAbove", "ThirdBelow", "SixthAbove", "SixthBelow"];

export function intervalLabel(i: Interval): string {
  switch (i) {
    case "ThirdAbove":
      return "Third above";
    case "ThirdBelow":
      return "Third below";
    case "SixthAbove":
      return "Sixth above";
    case "SixthBelow":
      return "Sixth below";
  }
}

// ---- Generation ------------------------------------------------------------------------------

export type TaskKind = GenerationTask["kind"];

export const TASKS: readonly { id: TaskKind; label: string }[] = [
  { id: "Backing", label: "Backing for the vocal" },
  { id: "Lego", label: "One instrument" },
  { id: "Repaint", label: "Repaint a section" },
];

export const INSTRUMENTS: readonly Instrument[] = ["Drums", "Bass", "Guitar", "Keyboard", "Strings", "Synth", "Percussion", "Brass", "Woodwinds", "BackingVocals"];

export function instrumentLabel(i: Instrument): string {
  return i === "BackingVocals" ? "Backing vocals" : i;
}

/** The task to send for the form's choices. A repaint covers `[start, end)` of the song. */
export function generationTask(kind: TaskKind, from: EditSource, instrument: Instrument, start: number, end: number): GenerationTask {
  switch (kind) {
    case "Backing":
      return { kind: "Backing", from };
    case "Lego":
      return { kind: "Lego", from, instrument };
    case "Repaint":
      return { kind: "Repaint", start: Math.min(start, end), end: Math.max(start, end) };
  }
}

/** A fresh random seed (kept with the generated track, so it can be reproduced). */
export function randomSeed(random: () => number = Math.random): number {
  return Math.floor(random() * 2 ** 31);
}

export function generatorLabel(s: GeneratorState): string {
  switch (s.kind) {
    case "NotInstalled":
      return "Not installed";
    case "Installing":
      return s.step;
    case "Ready":
      return "Ready";
    case "Running":
      return "Generating";
    case "Failed":
      return "Failed";
  }
}

/** What the panel offers for the generator in a state. */
export type GeneratorAction = "install" | "retryInstall" | "uninstall" | "busy";

export function generatorAction(s: GeneratorState): GeneratorAction {
  switch (s.kind) {
    case "NotInstalled":
      return "install";
    case "Failed":
      return s.installed ? "uninstall" : "retryInstall";
    case "Ready":
      return "uninstall";
    case "Installing":
    case "Running":
      return "busy";
  }
}

/** Whether a generation can start. */
export function canGenerate(s: GeneratorState): boolean {
  return s.kind === "Ready" || (s.kind === "Failed" && s.installed);
}
