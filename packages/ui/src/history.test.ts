import assert from "node:assert/strict";
import test from "node:test";
import type { MixParams } from "@workstation/engine-api";
import { emptyHistory, HISTORY_LIMIT, record, redo, stateFor, undo, type Command } from "./history.ts";

const mix = (gain: number): MixParams => ({ gain, pan: 0, mode: "Normal" });
const master = (before: number | null, after: number | null): Command => ({ kind: "Master", before, after });

test("undo and redo walk the history both ways; a new change drops the redo future", () => {
  let h = record(record(emptyHistory, master(null, 1)), master(1, 2));
  const u = undo(h)!;
  assert.deepEqual(stateFor(u.command, "undo"), 1);
  h = u.history;
  const r = redo(h)!;
  assert.deepEqual(stateFor(r.command, "redo"), 2);
  h = record(u.history, master(1, 3));
  assert.equal(redo(h), null);
  assert.equal(h.past.length, 2);
  assert.equal(undo(emptyHistory), null);
});

test("a change that changes nothing isn't recorded", () => {
  const h = record(emptyHistory, { kind: "Mix", track: 0, before: mix(1), after: mix(0.5) });
  assert.equal(h.past.length, 1);
  assert.equal(record(h, master(2, 2)), h);
  assert.equal(record(h, { kind: "Mix", track: 0, before: mix(1), after: mix(1) }), h);
});

test("the history keeps its newest steps", () => {
  let h = emptyHistory;
  for (let i = 0; i < HISTORY_LIMIT + 10; i++) h = record(h, master(i, i + 1));
  assert.equal(h.past.length, HISTORY_LIMIT);
  assert.deepEqual(h.past.at(-1), master(HISTORY_LIMIT + 9, HISTORY_LIMIT + 10));
});

test("pitch slider changes with one key are one step", () => {
  const e = (n: number) => ({ source: { kind: "Take", id: 1 }, notes: [], n }) as never;
  let h = record(emptyHistory, { kind: "PitchEdit", source: { kind: "Take", id: 1 }, before: e(0), after: e(1), key: "centre" });
  h = record(h, { kind: "PitchEdit", source: { kind: "Take", id: 1 }, before: e(1), after: e(2), key: "centre" });
  assert.equal(h.past.length, 1);
  assert.deepEqual(stateFor(h.past[0], "undo"), e(0));
  h = record(h, { kind: "PitchEdit", source: { kind: "Take", id: 1 }, before: e(2), after: e(3), key: null });
  assert.equal(h.past.length, 2);
});
