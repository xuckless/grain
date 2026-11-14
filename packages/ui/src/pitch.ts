// The Pitch view's state machines and pure helpers (AGENTS.md §7.1). The engine owns every pitch
// decision (snapping, curves, rendering); this drives selection, dragging, undo/redo, the view
// window and the render job.

import type { EditNote, EditSource, PitchEdit, Project } from "@workstation/engine-api";

// ---- Undo / redo ------------------------------------------------------------------------------

/** Most edit-list snapshots kept for undo. */
export const HISTORY_LIMIT = 100;

/** Edit-list snapshots. `key`: what made `present`; a change with the same key (a slider being
 * dragged) replaces it instead of adding a step. */
export type History = { past: PitchEdit[]; present: PitchEdit; future: PitchEdit[]; key: string | null };

export function historyOf(edit: PitchEdit): History {
  return { past: [], present: edit, future: [], key: null };
}

/** `next` becomes the present. `key` coalesces repeated changes of one control into one step. */
export function commit(h: History, next: PitchEdit, key: string | null = null): History {
  if (key !== null && key === h.key) return { ...h, present: next, future: [] };
  return { past: [...h.past, h.present].slice(-HISTORY_LIMIT), present: next, future: [], key };
}

export function undo(h: History): History | null {
  const prev = h.past.at(-1);
  if (!prev) return null;
  return { past: h.past.slice(0, -1), present: prev, future: [h.present, ...h.future], key: null };
}

export function redo(h: History): History | null {
  const next = h.future[0];
  if (!next) return null;
  return { past: [...h.past, h.present], present: next, future: h.future.slice(1), key: null };
}

// ---- Selection ------------------------------------------------------------------------------

export type Selection = { kind: "none" } | { kind: "notes"; ids: number[] };

/** Click selection: a plain click picks one note, shift/⌘ toggles it, a click on nothing clears. */
export function select(s: Selection, id: number | null, extend: boolean): Selection {
  if (id === null) return extend ? s : { kind: "none" };
  if (!extend) return { kind: "notes", ids: [id] };
  const ids = s.kind === "notes" ? s.ids : [];
  const next = ids.includes(id) ? ids.filter((i) => i !== id) : [...ids, id];
  return next.length === 0 ? { kind: "none" } : { kind: "notes", ids: next };
}

export function selectAll(edit: PitchEdit): Selection {
  return edit.notes.length === 0 ? { kind: "none" } : { kind: "notes", ids: edit.notes.map((n) => n.id) };
}

/** Selected ids still present in `edit` (notes merge away). */
export function selectedIds(s: Selection, edit: PitchEdit): number[] {
  if (s.kind === "none") return [];
  return s.ids.filter((id) => edit.notes.some((n) => n.id === id));
}

/** The notes an action applies to: the selection, or every note when nothing is selected. */
export function targetIds(s: Selection, edit: PitchEdit): number[] {
  const ids = selectedIds(s, edit);
  return ids.length > 0 ? ids : edit.notes.map((n) => n.id);
}

// ---- View window ------------------------------------------------------------------------------

/** What the canvas shows: song seconds `[from, to)` and MIDI `[lo, hi]`. */
export type Window = { from: number; to: number; lo: number; hi: number };

/** Shortest time span the view zooms to, seconds. */
export const MIN_SPAN_SECONDS = 1;
/** Pitch padding around the notes, semitones. */
export const PITCH_PAD = 4;

/** The first window for an edit: its notes' time and pitch range (padded), or the whole song. */
export function initialWindow(edit: PitchEdit, duration: number): Window {
  const notes = edit.notes;
  if (notes.length === 0) return { from: 0, to: Math.max(MIN_SPAN_SECONDS, duration), lo: 48, hi: 72 };
  const pitches = notes.flatMap((n) => [n.centre, n.pitch]);
  const from = Math.max(0, notes[0].out_start - 1);
  const to = Math.min(duration, Math.max(from + MIN_SPAN_SECONDS, Math.min(notes.at(-1)!.out_end + 1, from + 20)));
  return { from, to, lo: Math.floor(Math.min(...pitches)) - PITCH_PAD, hi: Math.ceil(Math.max(...pitches)) + PITCH_PAD };
}

/** The window zoomed by `factor` (<1 zooms in) around song time `at`, kept inside the song. */
export function zoom(w: Window, factor: number, at: number, duration: number): Window {
  const span = Math.min(duration, Math.max(MIN_SPAN_SECONDS, (w.to - w.from) * factor));
  const ratio = (at - w.from) / (w.to - w.from);
  const from = clamp(at - ratio * span, 0, Math.max(0, duration - span));
  return { ...w, from, to: from + span };
}

/** The window panned by `seconds` and `semitones`; returns the clamped window and how far the
 * pan went past the song's ends (seconds) for the rubber band. */
export function pan(w: Window, seconds: number, semitones: number, duration: number): { window: Window; over: number } {
  const span = w.to - w.from;
  const wanted = w.from + seconds;
  const from = clamp(wanted, 0, Math.max(0, duration - span));
  return { window: { from, to: from + span, lo: w.lo + semitones, hi: w.hi + semitones }, over: wanted - from };
}

export function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

// ---- Hit testing ------------------------------------------------------------------------------

/** Pixels from a note's edge that grab the edge (stretch) instead of the body. */
export const EDGE_PX = 6;
/** Half the height of a note's hit box, semitones. */
export const HIT_SEMITONES = 0.6;

export type Part = "body" | "start" | "end";
export type Hit = { id: number; part: Part };

/** The note (and part) under song time `seconds`, pitch `midi`; `pxPerSecond` sizes the edges. */
export function hitTest(edit: PitchEdit, seconds: number, midi: number, pxPerSecond: number): Hit | null {
  const edge = EDGE_PX / pxPerSecond;
  for (const n of edit.notes) {
    if (Math.abs(midi - n.pitch) > HIT_SEMITONES) continue;
    if (seconds < n.out_start - edge || seconds > n.out_end + edge) continue;
    if (n.out_end - n.out_start > 4 * edge) {
      if (seconds <= n.out_start + edge) return { id: n.id, part: "start" };
      if (seconds >= n.out_end - edge) return { id: n.id, part: "end" };
    }
    return { id: n.id, part: "body" };
  }
  return null;
}

// ---- Dragging -----------------------------------------------------------------------------

/** Pixels a press must travel before it becomes a drag (and picks pitch or time). */
export const DRAG_THRESHOLD_PX = 5;

/**
 * A pointer drag on the canvas. `pressing`: down on a note, direction not decided yet. Body
 * drags lock to pitch (vertical) or time (horizontal) once past the threshold; edges stretch.
 * Background drags pan the view. While held, everything follows the pointer directly (rule 14);
 * the engine applies the edit on release.
 */
export type Drag =
  | { kind: "idle" }
  | { kind: "pressing"; hit: Hit; x0: number; y0: number }
  | { kind: "pitch"; x0: number; y0: number; ids: number[]; semitones: number }
  | { kind: "time"; x0: number; y0: number; id: number; seconds: number }
  | { kind: "stretch"; x0: number; y0: number; id: number; edge: "start" | "end"; seconds: number }
  | { kind: "panning"; x0: number; y0: number; from: Window; overPx: number };

/** What a press at (`x0`, `y0`) on `hit` turns into once the pointer is at (`x`, `y`): null
 * while still under the threshold. */
export function decideDrag(hit: Hit, x0: number, y0: number, x: number, y: number, selection: number[]): Drag | null {
  const dx = x - x0, dy = y - y0;
  if (Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return null;
  if (hit.part !== "body") return { kind: "stretch", x0, y0, id: hit.id, edge: hit.part, seconds: 0 };
  if (Math.abs(dy) >= Math.abs(dx)) {
    const ids = selection.includes(hit.id) ? selection : [hit.id];
    return { kind: "pitch", x0, y0, ids, semitones: 0 };
  }
  return { kind: "time", x0, y0, id: hit.id, seconds: 0 };
}

/** A held drag with the pointer at (`x`, `y`), given the view's scale. */
export function advance(d: Drag, x: number, y: number, pxPerSecond: number, pxPerSemitone: number): Drag {
  switch (d.kind) {
    case "pitch":
      return { ...d, semitones: (d.y0 - y) / pxPerSemitone };
    case "time":
    case "stretch":
      return { ...d, seconds: (x - d.x0) / pxPerSecond };
    default:
      return d;
  }
}

/** How a note is drawn while dragged: its pitch and output span with the drag applied. */
export function dragged(n: EditNote, d: Drag): { pitch: number; start: number; end: number } {
  const base = { pitch: n.pitch, start: n.out_start, end: n.out_end };
  switch (d.kind) {
    case "pitch":
      return d.ids.includes(n.id) ? { ...base, pitch: n.pitch + d.semitones } : base;
    case "time":
      return d.id === n.id ? { ...base, start: n.out_start + d.seconds, end: n.out_end + d.seconds } : base;
    case "stretch":
      if (d.id !== n.id) return base;
      return d.edge === "start" ? { ...base, start: Math.min(n.out_end - 0.05, n.out_start + d.seconds) } : { ...base, end: Math.max(n.out_start + 0.05, n.out_end + d.seconds) };
    default:
      return base;
  }
}

// ---- Rendering and sources ------------------------------------------------------------------

/** Rendering the edits into a new take. `take`: the take it made. */
export type RenderState = { kind: "idle" } | { kind: "rendering" } | { kind: "rendered"; take: number } | { kind: "failed"; message: string };

/** Loading a source's view. */
export type Loading<T> = { kind: "loading" } | { kind: "ready"; value: T } | { kind: "failed"; message: string };

export type SourceOption = { id: string; label: string };

/** Sources the editor can open: the vocal stem (when the song has a melody) and every take. */
export function sourceOptions(project: Project): SourceOption[] {
  const vocal = project.analysis.reference_notes.length > 0 ? [{ id: "vocal", label: "Vocal stem" }] : [];
  return [...vocal, ...project.takes.map((t) => ({ id: `take:${t.id}`, label: t.name }))];
}

export function sourceId(s: EditSource): string {
  return s.kind === "VocalStem" ? "vocal" : `take:${s.id}`;
}

export function sourceFromId(id: string): EditSource {
  return id === "vocal" ? { kind: "VocalStem" } : { kind: "Take", id: Number(id.slice("take:".length)) };
}

/** The source opened first: the newest take that isn't an edited copy, else the vocal stem. */
export function defaultSource(project: Project): EditSource | null {
  const originals = project.takes.filter((t) => t.source.kind !== "Edited");
  const take = originals.at(-1) ?? project.takes.at(-1);
  if (take) return { kind: "Take", id: take.id };
  return project.analysis.reference_notes.length > 0 ? { kind: "VocalStem" } : null;
}

/** The newest rendered (edited) take of `source`, if any. */
export function editedVersion(project: Project, source: EditSource): number | null {
  const same = (s: EditSource) => s.kind === source.kind && (s.kind === "VocalStem" || (source.kind === "Take" && s.id === source.id));
  const edited = project.takes.filter((t) => t.source.kind === "Edited" && same(t.source.from));
  return edited.length > 0 ? edited[edited.length - 1].id : null;
}

/** Pitch classes set in `mask` (bit 0 = C). */
export function inScale(mask: number, midi: number): boolean {
  return (mask & (1 << (((Math.round(midi) % 12) + 12) % 12))) !== 0;
}

/** Readout of a note's pitch change, "+1.20 st". */
export function formatShift(semitones: number): string {
  const r = Math.round(semitones * 100) / 100;
  return r === 0 ? "0 st" : `${r > 0 ? "+" : "−"}${Math.abs(r).toFixed(2)} st`;
}
