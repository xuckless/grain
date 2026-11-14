// The Auto-tune view's state machines and pure helpers (AGENTS.md §7). The engine decides every
// pitch; this only drives the controls, the live monitor and "Apply to take".

import type { MonitorMode, MonitorStatus, PlayerCommand, Project, Scale, TunePreview } from "@workstation/engine-api";
import { setMix, takeTrack, vocalTrack } from "./takes.ts";

/** Live monitoring: off, switching (the microphone may be opening), on dry or tuned, or failed
 * (the reason shown; any toggle tries again). */
export type MonitorState =
  | { kind: "off" }
  | { kind: "switching"; to: MonitorMode }
  | { kind: "on"; status: MonitorStatus }
  | { kind: "failed"; message: string };

export type MonitorEvent = { kind: "request"; to: MonitorMode } | { kind: "switched"; status: MonitorStatus } | { kind: "fail"; message: string };

/** The next monitor state, or `null` when the event doesn't apply (one switch at a time). */
export function monitorNext(s: MonitorState, e: MonitorEvent): MonitorState | null {
  switch (e.kind) {
    case "request":
      return s.kind === "switching" ? null : { kind: "switching", to: e.to };
    case "switched":
      if (s.kind !== "switching") return null;
      return e.status.mode === "Off" ? { kind: "off" } : { kind: "on", status: e.status };
    case "fail":
      return s.kind === "switching" ? { kind: "failed", message: e.message } : null;
  }
}

/** The mode the monitor is in, or is heading to. */
export function monitorMode(s: MonitorState): MonitorMode {
  switch (s.kind) {
    case "on":
      return s.status.mode;
    case "switching":
      return s.to;
    case "off":
    case "failed":
      return "Off";
  }
}

/** Rendering a take tuned into a new take. `applied.take`: the new take. */
export type ApplyState =
  | { kind: "idle" }
  | { kind: "applying"; take: number }
  | { kind: "applied"; take: number }
  | { kind: "failed"; message: string };

export type ScaleChoice = { id: string; label: string };

/** The key/scale menu: the keys the analysis found (best first), chromatic, or custom notes. */
export function scaleChoices(project: Project): ScaleChoice[] {
  return [
    ...project.analysis.keys.map((k, i) => ({ id: `key:${i}`, label: i === 0 ? `${k.label} (song)` : k.label })),
    { id: "chromatic", label: "Chromatic" },
    { id: "custom", label: "Custom notes" },
  ];
}

/** The menu entry of `scale`. A key the analysis didn't list shows as the song's key's entry. */
export function scaleId(project: Project, scale: Scale): string {
  switch (scale.kind) {
    case "Chromatic":
      return "chromatic";
    case "Custom":
      return "custom";
    case "Key": {
      const i = project.analysis.keys.findIndex((k) => k.key.tonic === scale.key.tonic && k.key.mode === scale.key.mode);
      return `key:${Math.max(0, i)}`;
    }
  }
}

/** All twelve pitch classes. */
export const ALL_PITCH_CLASSES = 0x0fff;

/** The scale for menu entry `id`. Custom notes start from all twelve (or the current set). */
export function pickScale(id: string, project: Project, current: Scale): Scale {
  if (id === "chromatic") return { kind: "Chromatic" };
  if (id === "custom") return current.kind === "Custom" ? current : { kind: "Custom", pitch_classes: ALL_PITCH_CLASSES };
  const key = project.analysis.keys[Number(id.slice("key:".length))]?.key ?? project.analysis.keys[0].key;
  return { kind: "Key", key };
}

/** A custom set with pitch class `pc` (0 = C) toggled; the last note can't be removed. */
export function togglePitchClass(pitchClasses: number, pc: number): number {
  const next = pitchClasses ^ (1 << pc);
  return (next & ALL_PITCH_CLASSES) === 0 ? pitchClasses : next;
}

/** One octave of piano keys for the custom-notes picker: pitch class and whether it's black. */
export const KEYBOARD: readonly { pc: number; black: boolean }[] = Array.from({ length: 12 }, (_, pc) => ({
  pc,
  black: [1, 3, 6, 8, 10].includes(pc),
}));

/** A held target note in a preview: frames `[start, end)` pulled to `midi`. */
export type TargetRun = { start: number; end: number; midi: number };

/** The preview's targets as runs of one note. */
export function targetRuns(preview: TunePreview): TargetRun[] {
  const runs: TargetRun[] = [];
  preview.target.forEach((midi, i) => {
    if (midi === null) return;
    const last = runs[runs.length - 1];
    if (last && last.end === i && last.midi === midi) last.end = i + 1;
    else runs.push({ start: i, end: i + 1, midi });
  });
  return runs;
}

/** MIDI range to draw a preview over (padded), or `null` when nothing is voiced. */
export function pitchRange(preview: TunePreview): { lo: number; hi: number } | null {
  const all = [...preview.sung, ...preview.target].filter((m): m is number => m !== null);
  if (all.length === 0) return null;
  return { lo: Math.floor(Math.min(...all)) - 2, hi: Math.ceil(Math.max(...all)) + 2 };
}

/** Readout of a retune time: "Hard" at 0, else milliseconds. */
export function formatRetune(ms: number): string {
  return ms <= 0 ? "Hard" : `${Math.round(ms)} ms`;
}

/** Readout of a formant shift in semitones. */
export function formatSemitones(st: number): string {
  const r = Math.round(st * 10) / 10;
  return r === 0 ? "0 st" : `${r > 0 ? "+" : "−"}${Math.abs(r).toFixed(1)} st`;
}

/** Readout of a 0–1 amount as a percentage. */
export function formatPercent(v: number): string {
  return `${Math.round(v * 100)}%`;
}

/** What plays while comparing: the song as mixed, or a take in place of the song's vocal. */
export type Listen = { kind: "song" } | { kind: "take"; take: number };

/** Player commands for `listen`. Every take and the vocal start from their saved mix, so
 * switching between takes (A/B) or back to the song never leaves one unmuted. */
export function listenCommands(project: Project, listen: Listen): PlayerCommand[] {
  const v = vocalTrack(project);
  const saved = [
    ...(v >= 0 ? [setMix(v, project.tracks[v].mix)] : []),
    ...project.takes.map((t) => setMix(takeTrack(project, t.id), t.mix)),
  ];
  if (listen.kind === "song") return saved;
  // Without stems the vocal is the whole mix: keep it, and the take plays over it.
  const stems = project.tracks.some((t) => t.source.kind === "Stem");
  return [
    ...saved,
    ...(v >= 0 && stems ? [setMix(v, { ...project.tracks[v].mix, mode: "Muted" })] : []),
    setMix(takeTrack(project, listen.take), { gain: 1, pan: 0, mode: "Normal" }),
  ];
}

/** The newest tuned version of take `id`, if any. */
export function tunedVersion(project: Project, id: number): number | null {
  const tuned = project.takes.filter((t) => t.source.kind === "Tuned" && t.source.from === id);
  return tuned.length > 0 ? tuned[tuned.length - 1].id : null;
}
