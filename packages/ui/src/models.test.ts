// Run with `pnpm --filter @workstation/ui test` (Node's built-in runner and type stripping).
import assert from "node:assert/strict";
import { test } from "node:test";
import type { ModelEntry, ModelInfo, Transcription } from "@workstation/engine-api";
import { formatBytes, laneRows, modelAction, modelSize, progress, stateLabel, withState } from "./models.ts";

const info: ModelInfo = {
  id: "basic_pitch",
  task: "",
  distribution: "default",
  license: "Apache-2.0",
  license_note: "",
  source: "",
  files: [{ name: "basic_pitch.onnx", url: "https://raw.githubusercontent.com/x", sha256: "00", size: 230444 }],
  obtain: { kind: "Download" },
};

test("the panel offers what each state allows", () => {
  const e = (state: ModelEntry["state"], obtain = info.obtain, removable = true): ModelEntry => ({ info: { ...info, obtain }, state, removable });
  assert.equal(modelAction(e({ kind: "NotDownloaded" })), "download");
  assert.equal(modelAction(e({ kind: "Downloading", received: 1, total: 2 })), "busy");
  assert.equal(modelAction(e({ kind: "Verifying" })), "busy");
  assert.equal(modelAction(e({ kind: "Ready" })), "delete");
  // Weights installed outside the app's store (a development checkout) aren't the app's to delete.
  assert.equal(modelAction(e({ kind: "Ready" }, info.obtain, false)), "installed");
  assert.equal(modelAction(e({ kind: "Failed", message: "x failed its checksum" })), "retry");
  assert.equal(modelAction(e({ kind: "NotDownloaded" }, { kind: "Manual", note: "gated" })), "manual");
  assert.equal(modelAction(e({ kind: "NotDownloaded" }, { kind: "Build", command: "python" })), "build");
});

test("progress and labels", () => {
  assert.equal(progress({ kind: "Downloading", received: 50, total: 200 }), 0.25);
  assert.equal(progress({ kind: "Ready" }), null);
  assert.equal(stateLabel({ kind: "Downloading", received: 50, total: 200 }), "Downloading 25%");
  assert.equal(formatBytes(230444), "230 kB");
  assert.equal(formatBytes(929273003), "929 MB");
  assert.equal(formatBytes(1_290_000_000), "1.29 GB");
  assert.equal(modelSize(info), 230444);
  const list = withState([{ info, state: { kind: "NotDownloaded" }, removable: false }], "basic_pitch", { kind: "Verifying" });
  assert.deepEqual(list[0].state, { kind: "Verifying" });
});

test("lane rows: drums on fixed rows, pitches spread", () => {
  const drums: Transcription = { source: { kind: "Stem", stem: "Drums" }, method: "DrumClassifier", notes: [] };
  const row = laneRows(drums);
  assert.ok(row({ start: 0, end: 0.1, pitch: 42, velocity: 90 }) < row({ start: 0, end: 0.1, pitch: 36, velocity: 90 }));
  const bass: Transcription = {
    source: { kind: "Stem", stem: "Bass" },
    method: "BasicPitch",
    notes: [
      { start: 0, end: 1, pitch: 40, velocity: 90 },
      { start: 1, end: 2, pitch: 52, velocity: 90 },
    ],
  };
  const r = laneRows(bass);
  assert.ok(r(bass.notes[1]) < r(bass.notes[0]));
});
