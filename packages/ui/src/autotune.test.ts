// Run with `pnpm --filter @workstation/ui test` (Node's built-in runner and type stripping).
import assert from "node:assert/strict";
import { test } from "node:test";
import type { MonitorStatus, Project, TunePreview } from "@workstation/engine-api";
import {
  ALL_PITCH_CLASSES,
  monitorMode,
  monitorNext,
  pickScale,
  pitchRange,
  scaleChoices,
  scaleId,
  targetRuns,
  togglePitchClass,
  type MonitorState,
} from "./autotune.ts";

const project = {
  analysis: {
    keys: [
      { key: { tonic: 9, mode: "Minor" }, correlation: 0.8, label: "A minor" },
      { key: { tonic: 0, mode: "Major" }, correlation: 0.7, label: "C major" },
    ],
  },
} as unknown as Project;

const tuned: MonitorStatus = { mode: "Tuned", latency_ms: 45 };

test("monitor runs off → switching → on, one switch at a time", () => {
  let s: MonitorState = { kind: "off" };
  s = monitorNext(s, { kind: "request", to: "Tuned" })!;
  assert.deepEqual(s, { kind: "switching", to: "Tuned" });
  assert.equal(monitorMode(s), "Tuned");
  assert.equal(monitorNext(s, { kind: "request", to: "Dry" }), null);
  s = monitorNext(s, { kind: "switched", status: tuned })!;
  assert.deepEqual(s, { kind: "on", status: tuned });
  assert.equal(monitorNext(s, { kind: "switched", status: tuned }), null);
  s = monitorNext(monitorNext(s, { kind: "request", to: "Off" })!, { kind: "switched", status: { mode: "Off", latency_ms: 0 } })!;
  assert.deepEqual(s, { kind: "off" });
});

test("a monitor that can't open fails with the reason, and can be retried", () => {
  let s: MonitorState = monitorNext({ kind: "off" }, { kind: "request", to: "Dry" })!;
  s = monitorNext(s, { kind: "fail", message: "no microphone" })!;
  assert.deepEqual(s, { kind: "failed", message: "no microphone" });
  assert.equal(monitorMode(s), "Off");
  assert.equal(monitorNext(s, { kind: "request", to: "Dry" })?.kind, "switching");
});

test("the scale menu lists the song's keys, chromatic and custom", () => {
  const choices = scaleChoices(project);
  assert.deepEqual(
    choices.map((c) => c.label),
    ["A minor (song)", "C major", "Chromatic", "Custom notes"],
  );
  const key = pickScale("key:1", project, { kind: "Chromatic" });
  assert.deepEqual(key, { kind: "Key", key: { tonic: 0, mode: "Major" } });
  assert.equal(scaleId(project, key), "key:1");
  assert.equal(scaleId(project, { kind: "Chromatic" }), "chromatic");
  assert.deepEqual(pickScale("custom", project, key), { kind: "Custom", pitch_classes: ALL_PITCH_CLASSES });
  const custom = { kind: "Custom", pitch_classes: 0b101 } as const;
  assert.equal(pickScale("custom", project, custom), custom);
});

test("custom notes toggle, keeping at least one", () => {
  assert.equal(togglePitchClass(0b101, 2), 0b001);
  assert.equal(togglePitchClass(0b001, 1), 0b011);
  assert.equal(togglePitchClass(0b001, 0), 0b001);
});

test("preview targets group into held notes", () => {
  const preview: TunePreview = {
    hop_seconds: 0.01,
    sung: [60.2, 60.4, null, 61.8, 62.1],
    target: [60, 60, null, 62, 62],
    corrected: [60, 60, null, 62, 62],
  };
  assert.deepEqual(targetRuns(preview), [
    { start: 0, end: 2, midi: 60 },
    { start: 3, end: 5, midi: 62 },
  ]);
  assert.deepEqual(pitchRange(preview), { lo: 58, hi: 65 });
  assert.equal(pitchRange({ ...preview, sung: [null], target: [null] }), null);
});

