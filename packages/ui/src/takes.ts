// Takes in the Score view: the recording and calibration state machines, player tracks, and the
// backing a singer hears while recording (AGENTS.md §5, §9). Pure functions with no Svelte or
// host imports, so they are unit-tested under Node (`pnpm --filter @workstation/ui test`).
// Each `…Next` returns the next state, or `null` for an illegal transition (rejected).

import type { Accompaniment, Latency, MixParams, PlayerCommand, Project, Transport } from "@workstation/engine-api";

// ---------------------------------------------------------------------------------------------
// Recording: Idle → CountIn → Recording → Stopping → Scored | Failed.

/** `heard`: the transport has shown the count-in. Until then a poll may still report the
 * state from before it, which must not end the take. */
export type Recording =
  | { kind: "idle" }
  | { kind: "countIn"; heard: boolean }
  | { kind: "recording" }
  | { kind: "stopping" }
  | { kind: "scored"; take: number }
  | { kind: "failed"; message: string };

export type RecordingEvent =
  | { kind: "arm" }
  | { kind: "countInHeard" }
  | { kind: "countInElapsed" }
  | { kind: "stop" }
  | { kind: "cancel" }
  | { kind: "captured"; take: number }
  | { kind: "fail"; message: string };

export function recordingNext(s: Recording, e: RecordingEvent): Recording | null {
  switch (e.kind) {
    case "arm":
      return s.kind === "idle" || s.kind === "scored" || s.kind === "failed" ? { kind: "countIn", heard: false } : null;
    case "countInHeard":
      return s.kind === "countIn" && !s.heard ? { kind: "countIn", heard: true } : null;
    case "countInElapsed":
      return s.kind === "countIn" && s.heard ? { kind: "recording" } : null;
    case "stop":
      return s.kind === "countIn" || s.kind === "recording" ? { kind: "stopping" } : null;
    case "cancel":
      return s.kind === "countIn" || s.kind === "recording" ? { kind: "idle" } : null;
    case "captured":
      return s.kind === "stopping" ? { kind: "scored", take: e.take } : null;
    case "fail":
      return s.kind === "countIn" || s.kind === "recording" || s.kind === "stopping" ? { kind: "failed", message: e.message } : null;
  }
}

/** Whether the microphone is in use (the recording panel is showing). */
export function isCapturing(s: Recording): boolean {
  return s.kind === "countIn" || s.kind === "recording" || s.kind === "stopping";
}

/** What the transport says about a recording: the count-in ended, or the song stopped. */
export function observeRecording(s: Recording, t: Transport): RecordingEvent | null {
  const halted = t.kind === "Stopped" || t.kind === "Paused";
  if (s.kind === "countIn" && !s.heard) return t.kind === "CountIn" ? { kind: "countInHeard" } : null;
  if (s.kind === "countIn" && t.kind === "Playing") return { kind: "countInElapsed" };
  if ((s.kind === "countIn" || s.kind === "recording") && halted) return { kind: "stop" };
  return null;
}

/** Beats left in the count-in, for the countdown. */
export function beatsLeft(t: Transport, bpm: number, sampleRate: number): number {
  if (t.kind !== "CountIn") return 0;
  return Math.ceil(t.remaining / sampleRate / (60 / bpm));
}

// ---------------------------------------------------------------------------------------------
// Latency calibration.

export type Calibration =
  | { kind: "idle" }
  | { kind: "measuring" }
  | { kind: "measured"; latency: Latency }
  | { kind: "failed"; message: string };

export type CalibrationEvent =
  | { kind: "start" }
  | { kind: "done"; latency: Latency }
  | { kind: "fail"; message: string }
  | { kind: "dismiss" };

export function calibrationNext(s: Calibration, e: CalibrationEvent): Calibration | null {
  switch (e.kind) {
    case "start":
      return s.kind === "measuring" ? null : { kind: "measuring" };
    case "done":
      return s.kind === "measuring" ? { kind: "measured", latency: e.latency } : null;
    case "fail":
      return s.kind === "measuring" ? { kind: "failed", message: e.message } : null;
    case "dismiss":
      return s.kind === "measured" || s.kind === "failed" ? { kind: "idle" } : null;
  }
}

/** The quick calibration before each recording: Calibrating → Ready. `source`: whether the chirp
 * was heard (Measured), or the device's stored latency is used instead (Previous: calibrated or set
 * before; Estimated: never calibrated). Not hearing the chirp never blocks a recording. */
export type AutoCalibration =
  | { kind: "calibrating" }
  | { kind: "ready"; latency: Latency; source: "Measured" | "Previous" | "Estimated" };

/** The microphone calls the auto-calibration uses (the host's loopback test, unchanged). */
export type LatencyMeter = { measureLatency(): Promise<Latency>; latency(): Promise<Latency> };

/** Plays the loopback chirp and stores what it measures; when it isn't heard (headphones on,
 * volume low), keeps the latency stored for this device. */
export async function calibrateBeforeRecording(mic: LatencyMeter): Promise<Extract<AutoCalibration, { kind: "ready" }>> {
  try {
    return { kind: "ready", latency: await mic.measureLatency(), source: "Measured" };
  } catch {
    const latency = await mic.latency();
    return { kind: "ready", latency, source: latency.source.kind === "Default" ? "Estimated" : "Previous" };
  }
}

/** What the (i) popover says about a calibration that used a stored latency. */
export function calibrationNote(c: AutoCalibration): string | null {
  if (c.kind !== "ready" || c.source === "Measured") return null;
  const what = c.source === "Previous" ? "your last latency" : "an estimated latency";
  return `Couldn't hear the test sound before your last take (headphones on, or the volume low), so it used ${what} (${c.latency.ms.toFixed(0)} ms). To measure it, unplug your headphones or turn the volume up.`;
}

/** Step of the manual latency nudge, ms. */
export const LATENCY_NUDGE_MS = 5;

// ---------------------------------------------------------------------------------------------
// Player tracks and mixes.

/** Player index of the vocal stem (or the unseparated mix). */
export function vocalTrack(project: Project): number {
  const i = project.tracks.findIndex((t) => t.source.kind === "Stem" && t.source.stem === "Vocals");
  return i >= 0 ? i : project.tracks.findIndex((t) => t.source.kind === "Mix");
}

/** Player index of take `id`: takes follow the song's tracks. */
export function takeTrack(project: Project, id: number): number {
  return project.tracks.length + project.takes.findIndex((t) => t.id === id);
}

/** The project with player track `track`'s mix set (song tracks first, then takes). */
export function withMix(project: Project, track: number, params: MixParams): Project {
  const n = project.tracks.length;
  if (track < n) return { ...project, tracks: project.tracks.map((t, i) => (i === track ? { ...t, mix: params } : t)) };
  const k = track - n;
  return { ...project, takes: project.takes.map((t, i) => (i === k ? { ...t, mix: params } : t)) };
}

/** Copies `params`: project state may be a reactive proxy, which hosts can't post or clone. */
export function setMix(track: number, params: MixParams): PlayerCommand {
  return { kind: "SetMix", track, params: { mode: params.mode, gain: params.gain, pan: params.pan } };
}

// ---------------------------------------------------------------------------------------------
// Backing while recording.

export type Backing = "Instrumental" | "Guide" | "FullMix";

/** Level of the original vocal in the "guide vocal" backing (about −10 dB). */
export const GUIDE_GAIN = 0.3;

export type BackingOption = { id: Backing; label: string; available: boolean };

/** The backing choices for a song. Without stems the vocal can't be taken out. */
export function backingOptions(project: Project): BackingOption[] {
  const stems = vocalTrack(project) >= 0 && project.tracks.some((t) => t.source.kind === "Stem");
  return [
    { id: "Instrumental", label: "Instrumental", available: stems },
    { id: "Guide", label: "Instrumental + guide vocal", available: stems },
    { id: "FullMix", label: "Full mix", available: true },
  ];
}

/** The default backing, decided once from the stem levels measured at import or load. */
export function defaultBacking(a: Accompaniment): Backing {
  switch (a.kind) {
    case "Present":
      return "Instrumental";
    case "Absent":
      return "Guide";
    case "NoStems":
    case "Unmeasured":
      return "FullMix";
  }
}

/** Why the default is what it is, when that needs saying. */
export function backingNote(a: Accompaniment): string | null {
  return a.kind === "Absent" ? "This song has no backing track, so you'll hear the original vocal quietly." : null;
}

/** Commands that set the backing (only the vocal track changes). */
export function backingCommands(project: Project, backing: Backing): PlayerCommand[] {
  const v = vocalTrack(project);
  if (v < 0 || project.tracks[v].source.kind !== "Stem") return [];
  const mix = project.tracks[v].mix;
  switch (backing) {
    case "Instrumental":
      return [setMix(v, { ...mix, mode: "Muted" })];
    case "Guide":
      return [setMix(v, { ...mix, gain: mix.gain * GUIDE_GAIN })];
    case "FullMix":
      return [];
  }
}

/** Commands that put the vocal track back after recording. */
export function backingRestoreCommands(project: Project): PlayerCommand[] {
  const v = vocalTrack(project);
  return v >= 0 ? [setMix(v, project.tracks[v].mix)] : [];
}
