// Live karaoke (AGENTS.md §6.3–6.4): the session state machine and the performance it builds up
// from live frames (note fills, streaks, the running score). Frame verdicts come from the engine
// (§5's per-frame rule, `Microphone.live`); hit / near / miss come from `feedback.ts`. Pure
// functions with no Svelte or host imports, so they are unit-tested under Node.

import type { FrameTotals, LiveFrame, Note, OctaveMode, Transport } from "@workstation/engine-api";
import { frameFeedback, liveNoteFeedback, type Feedback } from "./feedback.ts";

// ---------------------------------------------------------------------------------------------
// Session: Idle → CountIn → Singing ⇄ Paused → Finished (+ Failed). Every uninterrupted run
// (count-in to pause or finish) is recorded and saved as a take, so pausing never loses singing.

export type Karaoke =
  | { kind: "idle" }
  /** `heard`: the transport has shown the count-in (an earlier poll may predate it). */
  | { kind: "countIn"; heard: boolean }
  | { kind: "singing" }
  /** The run is being saved before the pause takes hold. */
  | { kind: "pausing" }
  /** `frame`: where the song resumes. */
  | { kind: "paused"; frame: number }
  /** The last run is being saved. */
  | { kind: "finishing" }
  | { kind: "finished" }
  | { kind: "failed"; message: string };

export type KaraokeEvent =
  | { kind: "start" }
  | { kind: "countInHeard" }
  | { kind: "countInElapsed" }
  | { kind: "pause" }
  | { kind: "paused"; frame: number }
  | { kind: "resume" }
  | { kind: "finish" }
  | { kind: "saved" }
  | { kind: "reset" }
  | { kind: "fail"; message: string };

export function karaokeNext(s: Karaoke, e: KaraokeEvent): Karaoke | null {
  switch (e.kind) {
    case "start":
      return s.kind === "idle" || s.kind === "finished" || s.kind === "failed" ? { kind: "countIn", heard: false } : null;
    case "countInHeard":
      return s.kind === "countIn" && !s.heard ? { kind: "countIn", heard: true } : null;
    case "countInElapsed":
      return s.kind === "countIn" && s.heard ? { kind: "singing" } : null;
    case "pause":
      return s.kind === "singing" ? { kind: "pausing" } : null;
    case "paused":
      return s.kind === "pausing" ? { kind: "paused", frame: e.frame } : null;
    case "resume":
      return s.kind === "paused" ? { kind: "countIn", heard: false } : null;
    case "finish":
      if (s.kind === "countIn" || s.kind === "singing") return { kind: "finishing" };
      return s.kind === "paused" ? { kind: "finished" } : null;
    case "saved":
      return s.kind === "finishing" ? { kind: "finished" } : null;
    case "reset":
      return s.kind === "finished" || s.kind === "failed" ? { kind: "idle" } : null;
    case "fail":
      return s.kind === "countIn" || s.kind === "singing" || s.kind === "pausing" || s.kind === "finishing" ? { kind: "failed", message: e.message } : null;
  }
}

/** Whether a run is recording (the microphone is open). */
export function isLive(s: Karaoke): boolean {
  return s.kind === "countIn" || s.kind === "singing" || s.kind === "pausing" || s.kind === "finishing";
}

/** What the transport says: the count-in started or ended, or the song stopped (its end, or the
 * Stop button), which finishes the performance. */
export function observeKaraoke(s: Karaoke, t: Transport): KaraokeEvent | null {
  if (s.kind === "countIn" && !s.heard) return t.kind === "CountIn" ? { kind: "countInHeard" } : null;
  if (s.kind === "countIn" && t.kind === "Playing") return { kind: "countInElapsed" };
  if ((s.kind === "countIn" || s.kind === "singing") && (t.kind === "Stopped" || t.kind === "Paused")) return { kind: "finish" };
  return null;
}

// ---------------------------------------------------------------------------------------------
// Performance: what the live frames add up to.

const NO_TOTALS: FrameTotals = { hit: 0, miss: 0, reference_only: 0, user_only: 0 };

type Tally = { hit: number; near: number; voiced: number };

export type Performance = {
  /** Totals of finished runs, and the running totals of the current one (they reset per run). */
  done: FrameTotals;
  run: FrameTotals;
  /** Per reference note: frame counts, and its verdict once the song has passed it. */
  tallies: Tally[];
  fills: (Feedback | null)[];
  /** Notes tallied but not judged yet. */
  open: number[];
  streak: number;
  best: number;
  /** Takes saved by this performance's runs. */
  takes: number[];
};

export function newPerformance(notes: readonly Note[]): Performance {
  return {
    done: NO_TOTALS,
    run: NO_TOTALS,
    tallies: notes.map(() => ({ hit: 0, near: 0, voiced: 0 })),
    fills: notes.map(() => null),
    open: [],
    streak: 0,
    best: 0,
    takes: [],
  };
}

/** Index of the note sounding at `seconds`, or -1 (notes are sorted and don't overlap). */
export function noteAt(notes: readonly Note[], seconds: number): number {
  let lo = 0, hi = notes.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (notes[mid].end < seconds) lo = mid + 1;
    else if (notes[mid].start > seconds) hi = mid - 1;
    else return mid;
  }
  return -1;
}

/** Judges note `i` from its tally and updates the streak. */
function judge(p: Performance, i: number): Feedback {
  const t = p.tallies[i];
  const fill = liveNoteFeedback(t.hit, t.near, t.voiced);
  p.fills[i] = fill;
  p.streak = fill === "hit" ? p.streak + 1 : 0;
  p.best = Math.max(p.best, p.streak);
  return fill;
}

/** Adds a live update's frames; returns the notes judged by it (hits get a burst). Notes the
 * singer never reached (started later, or paused over) stay unjudged. */
export function addFrames(p: Performance, notes: readonly Note[], frames: readonly LiveFrame[], run: FrameTotals): { note: number; fill: Feedback }[] {
  p.run = run;
  const judged: { note: number; fill: Feedback }[] = [];
  for (const f of frames) {
    // Judge open notes the song has moved past, in order.
    while (p.open.length > 0 && notes[p.open[0]].end < f.seconds) {
      const i = p.open.shift()!;
      judged.push({ note: i, fill: judge(p, i) });
    }
    const i = noteAt(notes, f.seconds);
    if (i < 0 || p.fills[i] !== null) continue;
    const t = p.tallies[i];
    if (f.comparison.kind === "Hit" || f.comparison.kind === "Miss" || f.comparison.kind === "ReferenceOnly") t.voiced++;
    const fb = frameFeedback(f.comparison);
    if (fb === "hit") t.hit++;
    else if (fb === "near") t.near++;
    if (!p.open.includes(i)) p.open.push(i);
  }
  return judged;
}

/** Ends a run: judges what it left open and banks its totals. */
export function endRun(p: Performance, take: number | null): { note: number; fill: Feedback }[] {
  const judged = p.open.map((i) => ({ note: i, fill: judge(p, i) }));
  p.open = [];
  p.done = sumTotals(p.done, p.run);
  p.run = NO_TOTALS;
  if (take !== null) p.takes.push(take);
  return judged;
}

export function sumTotals(a: FrameTotals, b: FrameTotals): FrameTotals {
  return { hit: a.hit + b.hit, miss: a.miss + b.miss, reference_only: a.reference_only + b.reference_only, user_only: a.user_only + b.user_only };
}

/** The live score: share of the reference's sung moments the singer hit (0–100). */
export function inTune(t: FrameTotals): number {
  const due = t.hit + t.miss + t.reference_only;
  return due > 0 ? (100 * t.hit) / due : 0;
}

export function performanceTotals(p: Performance): FrameTotals {
  return sumTotals(p.done, p.run);
}

/** The sung pitch to draw against note `reference`: with octave folding, moved to the octave
 * nearest the reference note (a man singing a female part an octave down still lines up). */
export function displayMidi(sung: number, reference: number | null, octave: OctaveMode): number {
  if (octave === "Strict" || reference === null) return sung;
  return sung - 12 * Math.round((sung - reference) / 12);
}
