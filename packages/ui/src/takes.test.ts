// Run with `pnpm --filter @workstation/ui test` (Node's built-in runner and type stripping).
import assert from "node:assert/strict";
import { test } from "node:test";
import type { Project, Track } from "@workstation/engine-api";
import {
  auditionCommands,
  auditionEndCommands,
  auditionNext,
  backingCommands,
  backingNote,
  backingOptions,
  beatsLeft,
  calibrationNext,
  defaultBacking,
  observeAudition,
  observeRecording,
  recordingNext,
  type Audition,
  type Recording,
  type RecordingEvent,
} from "./takes.ts";

const mix = { mode: "Normal", gain: 1, pan: 0 } as const;

function track(stem: "Vocals" | "Drums" | null): Track {
  return { name: stem ?? "Mix", source: stem ? { kind: "Stem", stem } : { kind: "Mix" }, mix, peaks: [] } as unknown as Track;
}

function project(tracks: Track[]): Project {
  return {
    tracks,
    takes: [{ id: 1, name: "Take 1", mix: { ...mix, mode: "Muted" } }],
    analysis: { accompaniment: { kind: "Present", relative_db: -6 } },
  } as unknown as Project;
}

test("recording runs idle → countIn → recording → stopping → scored", () => {
  let s: Recording = { kind: "idle" };
  const steps: [RecordingEvent, Recording["kind"]][] = [
    [{ kind: "arm" }, "countIn"],
    [{ kind: "countInHeard" }, "countIn"],
    [{ kind: "countInElapsed" }, "recording"],
    [{ kind: "stop" }, "stopping"],
    [{ kind: "captured", take: 3 }, "scored"],
    [{ kind: "arm" }, "countIn"],
  ];
  for (const [e, kind] of steps) {
    const next = recordingNext(s, e);
    assert.ok(next, `${s.kind} --${e.kind}-->`);
    assert.equal(next.kind, kind);
    s = next;
  }
});

test("recording rejects illegal transitions", () => {
  assert.equal(recordingNext({ kind: "idle" }, { kind: "stop" }), null);
  assert.equal(recordingNext({ kind: "idle" }, { kind: "countInElapsed" }), null);
  assert.equal(recordingNext({ kind: "countIn", heard: false }, { kind: "countInElapsed" }), null);
  assert.equal(recordingNext({ kind: "recording" }, { kind: "arm" }), null);
  assert.equal(recordingNext({ kind: "recording" }, { kind: "captured", take: 1 }), null);
  assert.equal(recordingNext({ kind: "stopping" }, { kind: "cancel" }), null);
  assert.equal(recordingNext({ kind: "scored", take: 1 }, { kind: "fail", message: "x" }), null);
  assert.deepEqual(recordingNext({ kind: "stopping" }, { kind: "fail", message: "no mic" }), { kind: "failed", message: "no mic" });
});

test("the transport drives the count-in and the end of the song", () => {
  // A stale poll from before the cue doesn't end the take.
  assert.equal(observeRecording({ kind: "countIn", heard: false }, { kind: "Stopped" }), null);
  assert.equal(observeRecording({ kind: "countIn", heard: false }, { kind: "Playing", frame: 9 }), null);
  assert.deepEqual(observeRecording({ kind: "countIn", heard: false }, { kind: "CountIn", frame: 0, remaining: 10 }), { kind: "countInHeard" });
  assert.equal(observeRecording({ kind: "countIn", heard: true }, { kind: "CountIn", frame: 0, remaining: 10 }), null);
  assert.deepEqual(observeRecording({ kind: "countIn", heard: true }, { kind: "Playing", frame: 0 }), { kind: "countInElapsed" });
  assert.deepEqual(observeRecording({ kind: "recording" }, { kind: "Stopped" }), { kind: "stop" });
  assert.equal(observeRecording({ kind: "stopping" }, { kind: "Stopped" }), null);
  // 120 bpm at 48 kHz: 1.6 beats left shows as 2.
  assert.equal(beatsLeft({ kind: "CountIn", frame: 0, remaining: 38_400 }, 120, 48_000), 2);
});

test("calibration measures once at a time", () => {
  const latency = { ms: 40, source: { kind: "Measured", confidence: 0.9 }, devices: "a | b" } as const;
  const measuring = calibrationNext({ kind: "idle" }, { kind: "start" });
  assert.deepEqual(measuring, { kind: "measuring" });
  assert.equal(calibrationNext({ kind: "measuring" }, { kind: "start" }), null);
  assert.deepEqual(calibrationNext({ kind: "measuring" }, { kind: "done", latency }), { kind: "measured", latency });
  assert.equal(calibrationNext({ kind: "idle" }, { kind: "done", latency }), null);
  assert.deepEqual(calibrationNext({ kind: "measuring" }, { kind: "fail", message: "quiet" }), { kind: "failed", message: "quiet" });
  assert.deepEqual(calibrationNext({ kind: "failed", message: "quiet" }, { kind: "start" }), { kind: "measuring" });
  assert.equal(calibrationNext({ kind: "idle" }, { kind: "dismiss" }), null);
});

test("audition plays the reference, then the take, then stops", () => {
  const span = { start: 1, end: 2 };
  let s: Audition = { kind: "idle" };
  s = auditionNext(s, { kind: "start", span, take: 1 })!;
  assert.equal(s.kind, "reference");
  // A poll from before the seek (paused elsewhere) is not the end of the phase.
  assert.equal(observeAudition(s, { kind: "Paused", frame: 9000 }, 1000), null);
  assert.equal(auditionNext(s, { kind: "ended" }), null);
  assert.deepEqual(observeAudition(s, { kind: "Playing", frame: 1000 }, 1000), { kind: "playing" });
  s = auditionNext(s, { kind: "playing" })!;
  assert.equal(observeAudition(s, { kind: "Playing", frame: 1500 }, 1000), null);
  assert.deepEqual(observeAudition(s, { kind: "Playing", frame: 2000 }, 1000), { kind: "ended" });
  s = auditionNext(s, { kind: "ended" })!;
  assert.equal(s.kind, "take");
  s = auditionNext(s, { kind: "playing" })!;
  assert.deepEqual(observeAudition(s, { kind: "Stopped" }, 1000), { kind: "ended" });
  s = auditionNext(s, { kind: "ended" })!;
  assert.equal(s.kind, "idle");
  assert.equal(auditionNext(s, { kind: "ended" }), null);
  assert.equal(auditionNext(s, { kind: "cancel" }), null);
});

test("audition solos the vocal stem, then the take, and restores the mix", () => {
  const p = project([track("Vocals"), track("Drums")]);
  const span = { start: 1, end: 2 };
  const ref = auditionCommands(p, { kind: "reference", span, take: 1, playing: false }, 1000);
  assert.deepEqual(ref.at(-3), { kind: "SetMix", track: 0, params: { gain: 1, pan: 0, mode: "Soloed" } });
  assert.deepEqual(ref.at(-2), { kind: "Seek", frame: 1000 });
  const take = auditionCommands(p, { kind: "take", span, take: 1, playing: false }, 1000);
  // The take is player track 2 (after the two stems).
  assert.deepEqual(take.at(-3), { kind: "SetMix", track: 2, params: { gain: 1, pan: 0, mode: "Soloed" } });
  assert.deepEqual(auditionEndCommands(p, { kind: "take", span, take: 1, playing: false }), [
    { kind: "Pause" },
    { kind: "SetMix", track: 0, params: mix },
    { kind: "SetMix", track: 2, params: { ...mix, mode: "Muted" } },
  ]);
});

test("the backing default follows the measured accompaniment", () => {
  assert.equal(defaultBacking({ kind: "Present", relative_db: -8 }), "Instrumental");
  assert.equal(defaultBacking({ kind: "Absent", relative_db: -42 }), "Guide");
  assert.equal(defaultBacking({ kind: "NoStems" }), "FullMix");
  assert.match(backingNote({ kind: "Absent", relative_db: -42 }) ?? "", /no backing track/);
  assert.equal(backingNote({ kind: "Present", relative_db: -8 }), null);
});

test("backings change only the vocal stem", () => {
  const p = project([track("Vocals"), track("Drums")]);
  assert.deepEqual(backingCommands(p, "Instrumental"), [{ kind: "SetMix", track: 0, params: { ...mix, mode: "Muted" } }]);
  assert.deepEqual(backingCommands(p, "Guide"), [{ kind: "SetMix", track: 0, params: { ...mix, gain: 0.3 } }]);
  assert.deepEqual(backingCommands(p, "FullMix"), []);
  const unseparated = project([track(null)]);
  assert.deepEqual(backingCommands(unseparated, "Instrumental"), []);
  assert.deepEqual(
    backingOptions(unseparated).map((o) => o.available),
    [false, false, true],
  );
});
