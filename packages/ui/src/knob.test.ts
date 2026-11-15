import assert from "node:assert/strict";
import test from "node:test";
import { dragTo, gainToKnob, knobAngle, knobToGain, knobToPan, litDots, panToKnob, SWEEP_DEGREES } from "./knob.ts";

const close = (a: number, b: number, eps = 1e-9) => assert.ok(Math.abs(a - b) < eps, `${a} ≉ ${b}`);

test("the gain knob follows the fader law and round-trips", () => {
  assert.equal(gainToKnob(0), 0);
  close(gainToKnob(10 ** (6 / 20)), 1);
  close(gainToKnob(1), 60 / 66);
  for (const g of [0.002, 0.1, 0.5, 1, 1.5]) close(knobToGain(gainToKnob(g)), g, 1e-9);
  assert.equal(knobToGain(0), 0);
});

test("the pan knob is centred at C and spans L100 to R100", () => {
  assert.equal(panToKnob(0), 0.5);
  assert.equal(knobToPan(0), -1);
  assert.equal(knobToPan(1), 1);
  close(knobToPan(panToKnob(-0.3)), -0.3);
});

test("angles sweep 270° around 12 o'clock; dragging up turns it up, ⇧ is finer", () => {
  assert.equal(knobAngle(0), -SWEEP_DEGREES / 2);
  assert.equal(knobAngle(0.5), 0);
  assert.equal(knobAngle(1), SWEEP_DEGREES / 2);
  close(dragTo(0.5, -80, false), 1);
  close(dragTo(0.5, -80, true), 0.55);
  assert.equal(dragTo(0.5, 1000, false), 0);
});

test("dots light from the start for gain and from the centre for pan", () => {
  assert.deepEqual(litDots(0.5, 5, false), [true, true, true, false, false]);
  assert.deepEqual(litDots(0.5, 5, true), [false, false, true, false, false]);
  assert.deepEqual(litDots(0, 5, true), [true, true, true, false, false]);
});
