// Hit / near / miss: how a sung frame or note reads, shared by the Score graph and the karaoke
// lane so both use one set of rules. Frame verdicts come from the engine (§5's per-frame rule);
// this only maps them onto the three feedback colours.

import type { FrameComparison, NoteResult } from "@workstation/engine-api";

export type Feedback = "hit" | "near" | "miss";

/** A missed frame within this many cents still reads as "near". */
export const NEAR_CENTS = 100;
/** Fraction of a note sung in tune for it to count as a hit. */
export const HIT_RATIO = 0.6;

/** Feedback for one compared frame; `null` when nobody (or only one side) sang. */
export function frameFeedback(f: FrameComparison): Feedback | null {
  switch (f.kind) {
    case "Hit":
      return "hit";
    case "Miss":
      return Math.abs(f.cents) <= NEAR_CENTS ? "near" : "miss";
    case "ReferenceOnly":
    case "UserOnly":
    case "Silent":
      return null;
  }
}

/** Feedback for a note from the offline scorer. */
export function noteFeedback(result: NoteResult): Feedback {
  switch (result.kind) {
    case "Missed":
      return "miss";
    case "Sung":
      return result.hit_ratio >= HIT_RATIO ? "hit" : "near";
  }
}

/** Feedback for a note from live frame counts: `hit`/`near` frames among the `voiced` frames
 * where the reference sings inside the note. Matches {@link noteFeedback}'s thresholds. */
export function liveNoteFeedback(hit: number, near: number, voiced: number): Feedback {
  if (voiced > 0 && hit / voiced >= HIT_RATIO) return "hit";
  return hit + near > 0 ? "near" : "miss";
}

/** A sung frame within this many cents of the note is a hit, for previews without a scorer
 * (the Score view's reference trail); the scorer's own tolerance depends on difficulty. */
export const PREVIEW_HIT_CENTS = 50;

/** Feedback for a pitch against the note sounding under it (`null`: no note). */
export function pitchFeedback(midi: number, note: number | null): Feedback | null {
  if (note === null) return null;
  const cents = Math.abs(midi - note) * 100;
  return cents <= PREVIEW_HIT_CENTS ? "hit" : cents <= NEAR_CENTS ? "near" : "miss";
}
