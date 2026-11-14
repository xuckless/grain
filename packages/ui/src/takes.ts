// Takes in the Score view: the recording, calibration and A/B audition state machines, and the
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

/** Step of the manual latency nudge, ms. */
export const LATENCY_NUDGE_MS = 5;

// ---------------------------------------------------------------------------------------------
// A/B audition: a note's span from the reference vocal, then the same span from the take.

export type Span = { start: number; end: number };

/** `playing`: the transport has shown this phase playing inside its span (until then a poll may
 * still report the position from before the seek). */
export type Audition =
  | { kind: "idle" }
  | { kind: "reference"; span: Span; take: number; playing: boolean }
  | { kind: "take"; span: Span; take: number; playing: boolean };

export type AuditionEvent = { kind: "start"; span: Span; take: number } | { kind: "playing" } | { kind: "ended" } | { kind: "cancel" };

export function auditionNext(s: Audition, e: AuditionEvent): Audition | null {
  switch (e.kind) {
    case "start":
      return { kind: "reference", span: e.span, take: e.take, playing: false };
    case "playing":
      return s.kind !== "idle" && !s.playing ? { ...s, playing: true } : null;
    case "ended":
      if (s.kind === "idle" || !s.playing) return null;
      return s.kind === "reference" ? { kind: "take", span: s.span, take: s.take, playing: false } : { kind: "idle" };
    case "cancel":
      return s.kind === "idle" ? null : { kind: "idle" };
  }
}

/** A phase starts when playback is seen inside its span, and ends when it passes the end or stops. */
export function observeAudition(s: Audition, t: Transport, sampleRate: number): AuditionEvent | null {
  if (s.kind === "idle") return null;
  const inSpan = t.kind === "Playing" && t.frame >= Math.floor(s.span.start * sampleRate) && t.frame < s.span.end * sampleRate;
  if (!s.playing) return inSpan ? { kind: "playing" } : null;
  return inSpan ? null : { kind: "ended" };
}

/** Seconds of lead-in before a note so its attack is heard. */
const AUDITION_LEAD = 0.15;

/** The span auditioned for a note. */
export function noteSpan(start: number, end: number): Span {
  return { start: Math.max(0, start - AUDITION_LEAD), end: end + AUDITION_LEAD };
}

/** Player index of the vocal stem (or the unseparated mix). */
function vocalTrack(project: Project): number {
  const i = project.tracks.findIndex((t) => t.source.kind === "Stem" && t.source.stem === "Vocals");
  return i >= 0 ? i : project.tracks.findIndex((t) => t.source.kind === "Mix");
}

/** Player index of take `id`: takes follow the song's tracks. */
export function takeTrack(project: Project, id: number): number {
  return project.tracks.length + project.takes.findIndex((t) => t.id === id);
}

/** Copies `params`: project state may be a reactive proxy, which hosts can't post or clone. */
function setMix(track: number, params: MixParams): PlayerCommand {
  return { kind: "SetMix", track, params: { mode: params.mode, gain: params.gain, pan: params.pan } };
}

/** Saved mix of every track an audition touches, to put back afterwards. */
function restore(project: Project, take: number): PlayerCommand[] {
  const v = vocalTrack(project);
  const t = project.takes.find((x) => x.id === take);
  return [...(v >= 0 ? [setMix(v, project.tracks[v].mix)] : []), ...(t ? [setMix(takeTrack(project, take), t.mix)] : [])];
}

/** Player commands that enter audition state `s` (from any other phase). */
export function auditionCommands(project: Project, s: Audition, sampleRate: number): PlayerCommand[] {
  if (s.kind === "idle") return [{ kind: "Pause" }];
  const v = vocalTrack(project);
  const solo = s.kind === "reference" ? v : takeTrack(project, s.take);
  const base = { gain: 1, pan: 0 };
  return [
    { kind: "Pause" },
    ...restore(project, s.take),
    setMix(solo, { ...base, mode: "Soloed" }),
    { kind: "Seek", frame: Math.max(0, Math.floor(s.span.start * sampleRate)) },
    { kind: "Play" },
  ];
}

/** Commands that leave an audition: stop and put the mix back. */
export function auditionEndCommands(project: Project, s: Audition): PlayerCommand[] {
  return s.kind === "idle" ? [] : [{ kind: "Pause" }, ...restore(project, s.take)];
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
