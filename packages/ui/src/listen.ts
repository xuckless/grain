// Hearing a take (AGENTS.md ground rule 1, §5): the one app-wide audition state machine. There
// is one player, so one thing plays at a time: listening to a take stops the song, and playing
// the song (or another take) ends the listening. Pure, unit-tested under Node.
//
//   Idle → Playing(item, mode) ⇄ Paused(item, mode) → Idle
//
// A play request can queue items back to back (an original, then its tuned version), and can
// limit them to one span (Score's note A/B: the reference vocal's note, then the take's).

import type { EditSource, PlayerCommand, Project, Take, Transport } from "@workstation/engine-api";
import { backingCommands, setMix, takeTrack, vocalTrack, type Backing } from "./takes.ts";

/** What can be heard alone: a take, or the song's vocal stem. */
export type ListenItem = EditSource;

/** `Solo`: the item alone. `WithBacking`: over the backing chosen for recording. */
export type ListenMode = "Solo" | "WithBacking";

export type Span = { start: number; end: number };

/** `heard`: the transport has shown this item playing inside its span (until then a push may
 * still report the position from before the seek). `queue`: items still to play after it.
 * `span`: the part of every item played (`null`: each item's sung span). */
export type Listening =
  | { kind: "idle" }
  | { kind: "playing"; item: ListenItem; mode: ListenMode; queue: ListenItem[]; span: Span | null; heard: boolean }
  | { kind: "paused"; item: ListenItem; mode: ListenMode; queue: ListenItem[]; span: Span | null };

export type ListenEvent =
  | { kind: "play"; items: ListenItem[]; mode: ListenMode; span?: Span }
  | { kind: "heard" }
  | { kind: "ended" }
  | { kind: "pause" }
  | { kind: "resume" }
  | { kind: "stop" };

export function listenNext(s: Listening, e: ListenEvent): Listening | null {
  switch (e.kind) {
    case "play": {
      const [item, ...queue] = e.items;
      return item ? { kind: "playing", item, mode: e.mode, queue, span: e.span ?? null, heard: false } : null;
    }
    case "heard":
      return s.kind === "playing" && !s.heard ? { ...s, heard: true } : null;
    case "ended": {
      if (s.kind !== "playing" || !s.heard) return null;
      const [item, ...queue] = s.queue;
      return item ? { kind: "playing", item, mode: s.mode, queue, span: s.span, heard: false } : { kind: "idle" };
    }
    case "pause":
      return s.kind === "playing" ? { kind: "paused", item: s.item, mode: s.mode, queue: s.queue, span: s.span } : null;
    case "resume":
      return s.kind === "paused" ? { kind: "playing", item: s.item, mode: s.mode, queue: s.queue, span: s.span, heard: false } : null;
    case "stop":
      return s.kind === "idle" ? null : { kind: "idle" };
  }
}

export const sameItem = (a: ListenItem, b: ListenItem): boolean => a.kind === b.kind && (a.kind === "VocalStem" || (b.kind === "Take" && a.id === b.id));

/** Whether `item` is what's playing or paused. */
export function isListening(s: Listening, item: ListenItem): boolean {
  return s.kind !== "idle" && sameItem(s.item, item);
}

/** Seconds kept after the last sung frame, so a phrase's tail is heard. */
const TAIL = 0.4;

/** Last voiced moment of a pitch track, in song seconds (`null`: never voiced). */
function lastVoiced(t: Take["pitch"]): number | null {
  for (let i = t.frames.length - 1; i >= 0; i--) if (t.frames[i].kind === "Voiced") return i * t.hop_seconds;
  return null;
}

/** Where `item` is sung on the song timeline. */
export function itemSpan(project: Project, item: ListenItem): Span {
  const duration = project.frames / project.sample_rate;
  if (item.kind === "VocalStem") {
    const notes = project.analysis.reference_notes;
    return notes.length > 0 ? { start: notes[0].start, end: Math.min(duration, notes[notes.length - 1].end + TAIL) } : { start: 0, end: duration };
  }
  const take = project.takes.find((t) => t.id === item.id);
  if (!take) return { start: 0, end: 0 };
  const last = lastVoiced(take.pitch);
  return { start: take.start_seconds, end: last === null ? duration : Math.min(duration, Math.max(take.start_seconds, last) + TAIL) };
}

/** The song as mixed: the vocal and every take back on their saved mix. */
export function songMix(project: Project): PlayerCommand[] {
  const v = vocalTrack(project);
  return [...(v >= 0 ? [setMix(v, project.tracks[v].mix)] : []), ...project.takes.map((t) => setMix(takeTrack(project, t.id), t.mix))];
}

const FULL = { gain: 1, pan: 0 };

/** Seconds of lead-in before a note so its attack is heard. */
const NOTE_LEAD = 0.15;

/** The span auditioned for a note (Score's A/B). */
export function noteSpan(start: number, end: number): Span {
  return { start: Math.max(0, start - NOTE_LEAD), end: end + NOTE_LEAD };
}

/** The mix that plays `item` in `mode`, from the song's saved mix. */
function itemMix(project: Project, item: ListenItem, mode: ListenMode, backing: Backing): PlayerCommand[] {
  const track = item.kind === "VocalStem" ? vocalTrack(project) : takeTrack(project, item.id);
  if (track < 0) return [];
  if (mode === "Solo") return [setMix(track, { ...FULL, mode: "Soloed" })];
  // Over the backing: the stem plays as the song does; a take replaces the song's vocal as the
  // recording backing does (muted, or quiet as a guide).
  return item.kind === "VocalStem" ? [] : [...backingCommands(project, backing), setMix(track, { ...FULL, mode: "Normal" })];
}

/** Player commands that move from `prev` to `next`. */
export function listenCommands(project: Project, prev: Listening, next: Listening, backing: Backing, sampleRate: number): PlayerCommand[] {
  switch (next.kind) {
    case "idle":
      return prev.kind === "idle" ? [] : [{ kind: "Pause" }, ...songMix(project)];
    case "paused":
      return [{ kind: "Pause" }];
    case "playing": {
      if (prev.kind === "paused" && prev.item === next.item) return [{ kind: "Play" }];
      if (prev.kind === "playing" && prev.item === next.item) return [];
      const span = next.span ?? itemSpan(project, next.item);
      return [
        { kind: "Pause" },
        ...songMix(project),
        ...itemMix(project, next.item, next.mode, backing),
        { kind: "Seek", frame: Math.max(0, Math.floor(span.start * sampleRate)) },
        { kind: "Play" },
      ];
    }
  }
}

/** What the transport says: the item started playing, or it passed its end or stopped. */
export function observeListening(project: Project, s: Listening, t: Transport, sampleRate: number): ListenEvent | null {
  if (s.kind !== "playing") return null;
  const span = s.span ?? itemSpan(project, s.item);
  const at = t.kind === "Playing" ? t.frame / sampleRate : null;
  const inSpan = at !== null && at >= span.start - 0.05 && at < span.end;
  if (!s.heard) return inSpan ? { kind: "heard" } : null;
  return inSpan ? null : { kind: "ended" };
}

/** 0–1 through what's playing at song time `seconds`. */
export function listenProgress(project: Project, s: Exclude<Listening, { kind: "idle" }>, seconds: number): number {
  const span = s.span ?? itemSpan(project, s.item);
  return Math.max(0, Math.min(1, (seconds - span.start) / Math.max(1e-6, span.end - span.start)));
}

// ---------------------------------------------------------------------------------------------
// Which takes are listed.

/** Sung takes (recorded, imported, tuned, edited, cleaned, chained): what take lists and menus
 * show. Harmony and generated tracks are parts of the arrangement and live in Mix (mirrors
 * engine-core `TakeSource::is_sung`). */
export function sungTakes(project: Project): Take[] {
  return project.takes.filter((t) => t.source.kind !== "Harmony" && t.source.kind !== "Generated");
}

/** The item a take was made from (tuned, edited, cleaned or chained), to compare against. */
export function madeFrom(take: Take): ListenItem | null {
  const s = take.source;
  switch (s.kind) {
    case "Tuned":
      return { kind: "Take", id: s.from };
    case "Edited":
    case "Denoised":
    case "Chained":
    case "Harmony":
      return s.from;
    case "Recorded":
    case "Imported":
    case "Generated":
      return null;
  }
}
