// Run with `pnpm --filter @workstation/ui test`.
import assert from "node:assert/strict";
import { test } from "node:test";
import type { FrameComparison, FrameTotals, LiveFrame, Note, Transport } from "@workstation/engine-api";
import { addFrames, displayMidi, endRun, inTune, karaokeNext, newPerformance, noteAt, observeKaraoke, performanceTotals, type Karaoke, type KaraokeEvent } from "./karaoke.ts";

function walk(events: KaraokeEvent[], from: Karaoke = { kind: "idle" }): Karaoke {
  return events.reduce<Karaoke>((s, e) => {
    const next = karaokeNext(s, e);
    assert.ok(next, `${e.kind} is legal from ${s.kind}`);
    return next;
  }, from);
}

test("karaoke session: count-in, sing, pause, resume, finish", () => {
  const singing = walk([{ kind: "start" }, { kind: "countInHeard" }, { kind: "countInElapsed" }]);
  assert.deepEqual(singing, { kind: "singing" });
  const paused = walk([{ kind: "pause" }, { kind: "paused", frame: 4410 }], singing);
  assert.deepEqual(paused, { kind: "paused", frame: 4410 });
  assert.deepEqual(walk([{ kind: "resume" }], paused), { kind: "countIn", heard: false });
  assert.deepEqual(walk([{ kind: "finish" }], paused), { kind: "finished" }, "the last run was saved when pausing");
  assert.deepEqual(walk([{ kind: "finish" }, { kind: "saved" }, { kind: "reset" }], singing), { kind: "idle" });
});

test("karaoke session rejects illegal transitions", () => {
  const idle: Karaoke = { kind: "idle" };
  assert.equal(karaokeNext(idle, { kind: "pause" }), null);
  assert.equal(karaokeNext(idle, { kind: "finish" }), null);
  assert.equal(karaokeNext({ kind: "countIn", heard: false }, { kind: "countInElapsed" }), null, "count-in must be heard first");
  assert.equal(karaokeNext({ kind: "singing" }, { kind: "start" }), null);
  assert.equal(karaokeNext({ kind: "pausing" }, { kind: "resume" }), null);
  assert.equal(karaokeNext({ kind: "finished" }, { kind: "fail", message: "x" }), null);
  assert.deepEqual(karaokeNext({ kind: "singing" }, { kind: "fail", message: "mic" }), { kind: "failed", message: "mic" });
});

test("the transport drives the count-in and the song's end", () => {
  const t = (kind: Transport["kind"]): Transport => ({ kind, frame: 0, remaining: 100 }) as unknown as Transport;
  assert.deepEqual(observeKaraoke({ kind: "countIn", heard: false }, t("Playing")), null, "a poll from before the count-in");
  assert.deepEqual(observeKaraoke({ kind: "countIn", heard: false }, t("CountIn")), { kind: "countInHeard" });
  assert.deepEqual(observeKaraoke({ kind: "countIn", heard: true }, t("Playing")), { kind: "countInElapsed" });
  assert.deepEqual(observeKaraoke({ kind: "singing" }, t("Stopped")), { kind: "finish" });
  assert.equal(observeKaraoke({ kind: "pausing" }, t("Stopped")), null);
});

const notes: Note[] = [
  { start: 1, end: 2, midi: 60 },
  { start: 3, end: 4, midi: 62 },
  { start: 5, end: 6, midi: 64 },
];

const frame = (seconds: number, comparison: FrameComparison): LiveFrame => ({ seconds, pitch: { kind: "Voiced", midi: 60 }, comparison });

function framesOver(start: number, end: number, comparison: FrameComparison): LiveFrame[] {
  const out: LiveFrame[] = [];
  const steps = Math.round((end - start) / 0.01);
  for (let k = 0; k <= steps; k++) out.push(frame(start + (k * (end - start)) / steps, comparison));
  return out;
}

test("notes fill as the song passes them, with a streak", () => {
  const p = newPerformance(notes);
  const totals: FrameTotals = { hit: 0, miss: 0, reference_only: 0, user_only: 0 };
  const hit: FrameComparison = { kind: "Hit", cents: 3 };
  assert.deepEqual(addFrames(p, notes, framesOver(1, 2, hit), totals), [], "not judged until the song moves past it");
  const judged = addFrames(p, notes, [...framesOver(2.01, 3, { kind: "Silent" }), ...framesOver(3, 4, { kind: "Miss", cents: 60 })], totals);
  assert.deepEqual(judged, [{ note: 0, fill: "hit" }]);
  assert.equal(p.streak, 1);
  assert.deepEqual(addFrames(p, notes, framesOver(4.01, 4.5, { kind: "Silent" }), totals), [{ note: 1, fill: "near" }]);
  assert.equal(p.streak, 0);
  assert.equal(p.best, 1);
  // Note 2 was never reached: ending the run leaves it unjudged.
  assert.deepEqual(endRun(p, 7), []);
  assert.deepEqual(p.fills, ["hit", "near", null]);
  assert.deepEqual(p.takes, [7]);
});

test("runs add up and a missed note reads as a miss", () => {
  const p = newPerformance(notes);
  addFrames(p, notes, framesOver(1, 2, { kind: "ReferenceOnly" }), { hit: 0, miss: 0, reference_only: 100, user_only: 0 });
  assert.deepEqual(endRun(p, null), [{ note: 0, fill: "miss" }]);
  addFrames(p, notes, framesOver(5, 6, { kind: "Hit", cents: 0 }), { hit: 100, miss: 0, reference_only: 0, user_only: 0 });
  assert.deepEqual(performanceTotals(p), { hit: 100, miss: 0, reference_only: 100, user_only: 0 });
  assert.equal(inTune(performanceTotals(p)), 50);
  assert.equal(inTune({ hit: 0, miss: 0, reference_only: 0, user_only: 3 }), 0);
});

test("finds notes and folds octaves for display", () => {
  assert.equal(noteAt(notes, 0.5), -1);
  assert.equal(noteAt(notes, 3.5), 1);
  assert.equal(noteAt(notes, 6), 2);
  assert.equal(displayMidi(48.2, 60, "Fold"), 60.2);
  assert.equal(displayMidi(71, 60, "Fold"), 59);
  assert.equal(displayMidi(48.2, 60, "Strict"), 48.2);
  assert.equal(displayMidi(48.2, null, "Fold"), 48.2);
});
