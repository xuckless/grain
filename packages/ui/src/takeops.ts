// Deleting and retaking takes (AGENTS.md §8.7): the one way these reach the host and the undo
// history, shared by the takes list, the toolbar's take picker, Karaoke's Retake and ⌘Z. A
// deleted or replaced take is set aside by the host, not destroyed, so undo brings it back.
// Pure over a `TakeSession` (the Session class satisfies it), so it is unit-tested under Node.

import type { Project, TakeStore } from "@workstation/engine-api";
import { stateFor, type Command, type Direction } from "./history.ts";
import type { ScoreState } from "./state.ts";

/** What these operations touch on the session. */
export type TakeSession = {
  project: Project;
  take: number | null;
  score: ScoreState;
  saved(project: Project): void;
  record(c: Command): void;
};

export type TakeCommand = Extract<Command, { kind: "Delete" } | { kind: "Replace" }>;

/** Views stop pointing at a take that left the song (the picker falls back to the master). */
function forget(s: TakeSession, id: number): void {
  if (s.score.kind === "scored" && s.score.take === id) s.score = { kind: "none" };
  if (s.take === id) s.take = null;
}

/** Views follow a take's slot to the take that now holds it. */
function follow(s: TakeSession, from: number, to: number): void {
  if (s.take === from) s.take = to;
}

/** Deletes take `id`; ⌘Z restores it. A deleted ★ master falls back to the newest take. */
export async function deleteTake(store: TakeStore, s: TakeSession, id: number): Promise<void> {
  s.saved(await store.remove(id));
  s.record({ kind: "Delete", take: id, before: "kept", after: "deleted" });
  forget(s, id);
}

/** A saved retake `replacement` takes over `target`'s place (slot, name, ★); ⌘Z swaps back. */
export async function replaceTake(store: TakeStore, s: TakeSession, target: number, replacement: number): Promise<void> {
  s.saved(await store.replace(target, replacement));
  s.record({ kind: "Replace", before: target, after: replacement });
  follow(s, target, replacement);
}

/** Undoes or redoes a delete or a retake. */
export async function applyTakeCommand(store: TakeStore, s: TakeSession, c: TakeCommand, direction: Direction): Promise<void> {
  if (c.kind === "Delete") {
    // Undo puts the take back (`before`: kept); redo deletes it again.
    if (direction === "undo") {
      s.saved(await store.restore(c.take));
    } else {
      s.saved(await store.remove(c.take));
      forget(s, c.take);
    }
    return;
  }
  const into = stateFor(c, direction);
  const out = into === c.after ? c.before : c.after;
  s.saved(await store.replace(out, into));
  follow(s, out, into);
}
