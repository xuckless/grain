import assert from "node:assert/strict";
import test from "node:test";
import { clampView, fitView, isFit, MIN_SPAN_SECONDS, panBy, timeToX, xToTime, zoomAt } from "./viewport.ts";

const D = 200;
const close = (a: number, b: number) => assert.ok(Math.abs(a - b) < 1e-9, `${a} ≉ ${b}`);

test("the shared viewport fits the song and maps time to x both ways", () => {
  const v = fitView(D);
  assert.ok(isFit(v, D));
  close(timeToX(v, 100, 1000), 500);
  close(xToTime(v, 250, 1000), 50);
});

test("zooming keeps the time under the anchor still", () => {
  const v = zoomAt(fitView(D), 0.25, 0.5, D);
  close(v.span, 50);
  close(v.start + 0.5 * v.span, 100);
  assert.ok(!isFit(v, D));
});

test("zoom stops at the minimum span and at the whole song", () => {
  assert.equal(zoomAt(fitView(D), 1e-6, 0, D).span, MIN_SPAN_SECONDS);
  assert.deepEqual(zoomAt({ start: 50, span: 50, overshoot: 0 }, 100, 0.5, D), { start: 0, span: D, overshoot: 0 });
});

test("panning stays inside the song and rubber-bands past its ends", () => {
  const v = { start: 10, span: 20, overshoot: 0 };
  close(panBy(v, 5, D).start, 15);
  const past = panBy(v, -50, D);
  assert.equal(past.start, 0);
  assert.ok(past.overshoot < 0);
  assert.ok(Math.abs(past.overshoot) < 20 * 0.06 + 1e-9);
  assert.equal(clampView({ start: 500, span: 20, overshoot: 0 }, D).start, D - 20);
});
