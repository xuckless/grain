// One session-wide undo/redo history (AGENTS.md §8.6): ⌘Z / ⇧⌘Z step through mix changes, take
// changes (master, rename, delete, retake), pitch edits and processing-chain edits in the order they were made.
// Commands are plain data with both states, so undo and redo are the same operation pointed
// either way; SessionScreen executes them against the host.

import type { EditSource, MixParams, PitchEdit, TakeChain } from "@workstation/engine-api";

export type Command =
  /** A track's mix (gain, pan, mute/solo). A drag is one command, from its start value. */
  | { kind: "Mix"; track: number; before: MixParams; after: MixParams }
  /** Which take is the master (★). */
  | { kind: "Master"; before: number | null; after: number | null }
  | { kind: "Rename"; take: number; before: string; after: string }
  /** A take deleted (the host keeps it aside until the song closes, so undo restores it). */
  | { kind: "Delete"; take: number; before: "kept"; after: "deleted" }
  /** A retake: which take holds the slot (`before`, the original; `after`, the retake). */
  | { kind: "Replace"; before: number; after: number }
  /** A pitch-editor edit list (the Pitch Edits step of a take's chain). `key`: one slider drag
   * (consecutive changes with the same key are one step). */
  | { kind: "PitchEdit"; source: EditSource; before: PitchEdit; after: PitchEdit; key: string | null }
  /** One step of a take's processing chain (Auto-tune, Clean up, Vocal chain), or its bypass. */
  | { kind: "Chain"; take: number; before: TakeChain; after: TakeChain; label: string };

export type Direction = "undo" | "redo";

/** The state a command sets when applied in `direction`. */
export function stateFor<C extends Command>(c: C, direction: Direction): C["after"] {
  return direction === "undo" ? c.before : c.after;
}

export type History = { past: readonly Command[]; future: readonly Command[] };

export const emptyHistory: History = { past: [], future: [] };

/** Most steps kept; older ones fall off. */
export const HISTORY_LIMIT = 200;

/** A new command: it joins the past, and the redo future is dropped. A command that changes
 * nothing is ignored. Callers record a drag once, when it ends (one drag, one step); pitch-editor
 * slider changes that share a key merge. */
export function record(h: History, c: Command): History {
  if (JSON.stringify(c.before) === JSON.stringify(c.after)) return h;
  const last = h.past.at(-1);
  if (c.kind === "PitchEdit" && c.key !== null && last?.kind === "PitchEdit" && last.key === c.key) {
    return { past: [...h.past.slice(0, -1), { ...last, after: c.after }], future: [] };
  }
  return { past: [...h.past, c].slice(-HISTORY_LIMIT), future: [] };
}

/** Undo: the command to apply backwards and the history after it (null when nothing to undo). */
export function undo(h: History): { command: Command; history: History } | null {
  const command = h.past[h.past.length - 1];
  if (!command) return null;
  return { command, history: { past: h.past.slice(0, -1), future: [command, ...h.future] } };
}

/** Redo: the command to apply forwards and the history after it (null when nothing to redo). */
export function redo(h: History): { command: Command; history: History } | null {
  const command = h.future[0];
  if (!command) return null;
  return { command, history: { past: [...h.past, command], future: h.future.slice(1) } };
}

/** What ⌘Z would undo, for its tooltip. */
export function describe(c: Command): string {
  switch (c.kind) {
    case "Mix":
      return "mix change";
    case "Master":
      return "master take";
    case "Rename":
      return "rename";
    case "Delete":
      return "take delete";
    case "Replace":
      return "retake";
    case "PitchEdit":
      return "pitch edit";
    case "Chain":
      return c.label;
  }
}

