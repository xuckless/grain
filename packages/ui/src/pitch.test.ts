// Run with `pnpm --filter @workstation/ui test` (Node's built-in runner and type stripping).
import assert from "node:assert/strict";
import { test } from "node:test";
import type { EditNote, PitchEdit } from "@workstation/engine-api";
import { advance, decideDrag, dragged, hitTest, inScale, pan, select, targetIds, zoom } from "./pitch.ts";

function note(id: number, start: number, end: number, pitch: number): EditNote {
  return { id, start, end, centre: pitch, pitch, drift: 1, modulation: 1, transition: 0, formant: 0, out_start: start, out_end: end };
}

function edit(notes: EditNote[]): PitchEdit {
  return { source: { kind: "VocalStem" }, scale: { kind: "Chromatic" }, notes };
}

const a = edit([note(1, 0, 1, 60), note(2, 1.5, 2, 64)]);

test("selection: click picks, shift toggles, empty click clears; tools fall back to all notes", () => {
  let s = select({ kind: "none" }, 1, false);
  assert.deepEqual(s, { kind: "notes", ids: [1] });
  s = select(s, 2, true);
  assert.deepEqual(s, { kind: "notes", ids: [1, 2] });
  s = select(s, 1, true);
  assert.deepEqual(s, { kind: "notes", ids: [2] });
  assert.deepEqual(select(s, null, false), { kind: "none" });
  assert.deepEqual(targetIds({ kind: "none" }, a), [1, 2]);
});

test("hit testing finds bodies and edges", () => {
  const pxPerSecond = 100;
  assert.deepEqual(hitTest(a, 0.5, 60.2, pxPerSecond), { id: 1, part: "body" });
  assert.deepEqual(hitTest(a, 0.99, 60, pxPerSecond), { id: 1, part: "end" });
  assert.deepEqual(hitTest(a, 1.52, 64, pxPerSecond), { id: 2, part: "start" });
  assert.equal(hitTest(a, 0.5, 62, pxPerSecond), null);
});

test("drags lock to an axis past the threshold and follow the pointer", () => {
  const hit = { id: 1, part: "body" as const };
  assert.equal(decideDrag(hit, 0, 0, 2, 2, []), null);
  const p = decideDrag(hit, 0, 0, 1, -10, [])!;
  assert.equal(p.kind, "pitch");
  const moved = advance(p, 1, -40, 100, 20);
  assert.equal(dragged(a.notes[0], moved).pitch, 62);
  const t = decideDrag(hit, 0, 0, 10, 1, [])!;
  assert.equal(t.kind, "time");
  assert.deepEqual(dragged(a.notes[0], advance(t, 50, 0, 100, 20)), { pitch: 60, start: 0.5, end: 1.5 });
  assert.equal(decideDrag({ id: 1, part: "end" }, 0, 0, 10, 0, [])!.kind, "stretch");
  // Dragging a selected note moves the whole selection.
  const many = decideDrag(hit, 0, 0, 0, 10, [1, 2])!;
  assert.deepEqual(many.kind === "pitch" && many.ids, [1, 2]);
});

test("pan reports how far it went past the song for the rubber band; zoom stays inside", () => {
  const w = { from: 0, to: 10, lo: 50, hi: 70 };
  const p = pan(w, -3, 0, 60);
  assert.equal(p.window.from, 0);
  assert.equal(p.over, -3);
  const z = zoom(w, 0.5, 5, 60);
  assert.deepEqual([z.from, z.to], [2.5, 7.5]);
  assert.equal(zoom(w, 100, 5, 60).to, 60);
});

test("scale masks", () => {
  const cMajor = 0b101010110101;
  assert.ok(inScale(cMajor, 60));
  assert.ok(!inScale(cMajor, 61));
  assert.ok(inScale(cMajor, 71));
});
