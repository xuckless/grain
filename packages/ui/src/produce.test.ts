// Run with `pnpm --filter @workstation/ui test` (Node's built-in runner and type stripping).
import assert from "node:assert/strict";
import { test } from "node:test";
import type { Measures, Project, Take } from "@workstation/engine-api";
import { canGenerate, describeOutput, generationTask, generatorAction, jobNext, meterRows, produceSource, producedTakes, randomSeed, type Job } from "./produce.ts";

test("a job runs once at a time and ends in one outcome", () => {
  let j: Job = { kind: "idle" };
  assert.equal(jobNext(j, { kind: "added", take: 1 }), null, "nothing to finish");
  j = jobNext(j, { kind: "start" })!;
  assert.equal(jobNext(j, { kind: "start" }), null, "already running");
  assert.deepEqual(jobNext(j, { kind: "needsModel", model: "deepfilternet3" }), { kind: "needsModel", model: "deepfilternet3" });
  j = jobNext(j, { kind: "fail", message: "x" })!;
  assert.equal(j.kind, "failed");
  j = jobNext(j, { kind: "start" })!;
  assert.deepEqual(jobNext(j, { kind: "added", take: 4 }), { kind: "done", take: 4 });
});

test("the generator offers what its state allows", () => {
  assert.equal(generatorAction({ kind: "NotInstalled" }), "install");
  assert.equal(generatorAction({ kind: "Installing", step: "Weights" }), "busy");
  assert.equal(generatorAction({ kind: "Running" }), "busy");
  assert.equal(generatorAction({ kind: "Ready" }), "uninstall");
  assert.equal(generatorAction({ kind: "Failed", message: "net", installed: false }), "retryInstall");
  assert.equal(generatorAction({ kind: "Failed", message: "oom", installed: true }), "uninstall");
  assert.ok(canGenerate({ kind: "Ready" }));
  assert.ok(canGenerate({ kind: "Failed", message: "oom", installed: true }));
  assert.ok(!canGenerate({ kind: "Running" }));
  assert.ok(!canGenerate({ kind: "NotInstalled" }));
});

test("generation tasks come from the form", () => {
  const from = { kind: "VocalStem" } as const;
  assert.deepEqual(generationTask("Backing", from, "Bass", 0, 0), { kind: "Backing", from });
  assert.deepEqual(generationTask("Lego", from, "Strings", 0, 0), { kind: "Lego", from, instrument: "Strings" });
  assert.deepEqual(generationTask("Repaint", from, "Bass", 20, 10), { kind: "Repaint", start: 10, end: 20 });
  const seed = randomSeed(() => 0.5);
  assert.equal(seed, 2 ** 30);
});

test("produced outputs list newest first with how they were made", () => {
  const take = (id: number, source: Take["source"]): Take => ({
    id,
    name: `t${id}`,
    source,
    start_seconds: 0,
    pitch_tracker: "Yin",
    pitch: { hop_seconds: 0.01, frames: [] },
    mix: { gain: 1, pan: 0, mode: "Muted" },
  });
  const request = { task: { kind: "Backing", from: { kind: "VocalStem" } }, prompt: "lofi", seed: 7, model: "acestep-v15-base", bpm: 90, key: "C major" } as const;
  const project = {
    takes: [take(1, { kind: "Recorded", latency_ms: 0 }), take(2, { kind: "Harmony", from: { kind: "Take", id: 1 }, interval: "ThirdAbove" }), take(3, { kind: "Generated", request })],
  } as unknown as Project;
  assert.deepEqual(producedTakes(project).map((t) => t.id), [3, 2]);
  assert.equal(describeOutput(producedTakes(project)[0]), "acestep-v15-base · seed 7 · “lofi”");
  assert.equal(describeOutput(producedTakes(project)[1]), "Third above, same voice");
  // Outputs never become the default source.
  assert.deepEqual(produceSource({ ...project, analysis: { reference_notes: [] } } as unknown as Project), { kind: "Take", id: 1 });
});

test("meters show before and after on fixed scales", () => {
  const m = (lufs: number): Measures => ({ lufs, peak_db: -6, tilt_db: -15, sibilance_db: -22 });
  const rows = meterRows(m(-30), m(-16));
  assert.equal(rows[0].before, "-30.0 LUFS");
  assert.equal(rows[0].after, "-16.0 LUFS");
  assert.ok(rows[0].afterFill > rows[0].beforeFill);
  assert.equal(meterRows(m(-30), null)[0].after, "");
});
