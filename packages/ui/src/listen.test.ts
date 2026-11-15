import assert from "node:assert/strict";
import test from "node:test";
import type { Project, Transport } from "@workstation/engine-api";
import { itemSpan, listenCommands, listenNext, madeFrom, noteSpan, observeListening, sungTakes, type Listening, type ListenEvent } from "./listen.ts";

const SR = 100;
const mix = { gain: 1, pan: 0, mode: "Normal" } as const;
const muted = { ...mix, mode: "Muted" } as const;
const voiced = (from: number, to: number, n: number) => ({
  hop_seconds: 0.1,
  frames: Array.from({ length: n }, (_, i) => (i >= from && i < to ? { kind: "Voiced", midi: 60 } : { kind: "Unvoiced" })),
});
const song = {
  sample_rate: SR,
  frames: 10 * SR,
  analysis: { reference_notes: [{ start: 1, end: 2, midi: 60 }] },
  tracks: [
    { source: { kind: "Stem", stem: "Vocals" }, mix },
    { source: { kind: "Stem", stem: "Drums" }, mix },
  ],
  takes: [
    { id: 1, mix: muted, start_seconds: 2, pitch: voiced(20, 40, 100), source: { kind: "Recorded", latency_ms: 0 } },
    { id: 2, mix: muted, start_seconds: 2, pitch: voiced(20, 40, 100), source: { kind: "Tuned", from: 1 } },
    { id: 3, mix, start_seconds: 0, pitch: voiced(0, 0, 0), source: { kind: "Harmony", from: { kind: "Take", id: 1 }, interval: "ThirdAbove" } },
    { id: 4, mix, start_seconds: 0, pitch: voiced(0, 0, 0), source: { kind: "Generated", request: {} } },
  ],
} as unknown as Project;

const take = (id: number) => ({ kind: "Take", id }) as const;

function run(s: Listening, ...events: ListenEvent[]): Listening {
  for (const e of events) s = listenNext(s, e) ?? assert.fail(`${e.kind} rejected in ${s.kind}`);
  return s;
}

test("one item plays at a time: a new play replaces the old, stop goes idle", () => {
  let s = run({ kind: "idle" }, { kind: "play", items: [take(1)], mode: "Solo" });
  assert.equal(s.kind, "playing");
  s = run(s, { kind: "play", items: [take(2)], mode: "WithBacking" });
  assert.deepEqual(s, { kind: "playing", item: take(2), mode: "WithBacking", queue: [], span: null, heard: false });
  assert.deepEqual(run(s, { kind: "stop" }), { kind: "idle" });
  assert.equal(listenNext({ kind: "idle" }, { kind: "stop" }), null);
  assert.equal(listenNext({ kind: "idle" }, { kind: "play", items: [], mode: "Solo" }), null);
});

test("back to back: the queue plays in order, then idle; pause and resume keep the item", () => {
  let s = run({ kind: "idle" }, { kind: "play", items: [take(1), take(2)], mode: "Solo" });
  // An end before the item was heard is a stale push: rejected.
  assert.equal(listenNext(s, { kind: "ended" }), null);
  s = run(s, { kind: "heard" }, { kind: "pause" });
  assert.equal(s.kind, "paused");
  s = run(s, { kind: "resume" }, { kind: "heard" }, { kind: "ended" });
  assert.deepEqual(s, { kind: "playing", item: take(2), mode: "Solo", queue: [], span: null, heard: false });
  assert.deepEqual(run(s, { kind: "heard" }, { kind: "ended" }), { kind: "idle" });
});

test("commands: solo seeks to the take and plays it alone; stopping puts the song mix back", () => {
  const idle: Listening = { kind: "idle" };
  const playing = run(idle, { kind: "play", items: [take(2)], mode: "Solo" });
  const songMix = [
    { kind: "SetMix", track: 0, params: mix },
    { kind: "SetMix", track: 2, params: muted },
    { kind: "SetMix", track: 3, params: muted },
    { kind: "SetMix", track: 4, params: mix },
    { kind: "SetMix", track: 5, params: mix },
  ];
  assert.deepEqual(listenCommands(song, idle, playing, "Instrumental", SR), [
    { kind: "Pause" },
    ...songMix,
    { kind: "SetMix", track: 3, params: { gain: 1, pan: 0, mode: "Soloed" } },
    { kind: "Seek", frame: 200 },
    { kind: "Play" },
  ]);
  const backed = run(idle, { kind: "play", items: [take(1)], mode: "WithBacking" });
  assert.deepEqual(listenCommands(song, idle, backed, "Instrumental", SR).slice(-4), [
    { kind: "SetMix", track: 0, params: muted },
    { kind: "SetMix", track: 2, params: mix },
    { kind: "Seek", frame: 200 },
    { kind: "Play" },
  ]);
  const paused = run(playing, { kind: "pause" });
  assert.deepEqual(listenCommands(song, playing, paused, "Instrumental", SR), [{ kind: "Pause" }]);
  assert.deepEqual(listenCommands(song, paused, run(paused, { kind: "resume" }), "Instrumental", SR), [{ kind: "Play" }]);
  assert.deepEqual(listenCommands(song, playing, { kind: "idle" }, "Instrumental", SR), [{ kind: "Pause" }, ...songMix]);
});

test("the transport starts and ends an item at its sung span", () => {
  const span = itemSpan(song, take(1));
  assert.equal(span.start, 2);
  assert.ok(Math.abs(span.end - 4.3) < 1e-9, `end ${span.end}`);
  assert.deepEqual(itemSpan(song, { kind: "VocalStem" }), { start: 1, end: 2.4 });
  const playingAt = (s: number): Transport => ({ kind: "Playing", frame: s * SR });
  let s = run({ kind: "idle" }, { kind: "play", items: [take(1)], mode: "Solo" });
  assert.equal(observeListening(song, s, playingAt(7), SR), null); // a push from before the seek
  assert.deepEqual(observeListening(song, s, playingAt(2), SR), { kind: "heard" });
  s = run(s, { kind: "heard" });
  assert.equal(observeListening(song, s, playingAt(3), SR), null);
  assert.deepEqual(observeListening(song, s, playingAt(4.5), SR), { kind: "ended" });
  assert.deepEqual(observeListening(song, s, { kind: "Paused", frame: 300 }, SR), { kind: "ended" });
});

test("take lists show sung takes only; derived takes know what they were made from", () => {
  assert.deepEqual(
    sungTakes(song).map((t) => t.id),
    [1, 2],
  );
  assert.deepEqual(madeFrom(song.takes[1]), take(1));
  assert.equal(madeFrom(song.takes[0]), null);
});

test("a note A/B: the reference vocal's note solo, then the take's, in one span", () => {
  const span = noteSpan(1.25, 1.85); // 1.1 s to 2.0 s
  let s = run({ kind: "idle" }, { kind: "play", items: [{ kind: "VocalStem" }, take(1)], mode: "Solo", span });
  const first = listenCommands(song, { kind: "idle" }, s, "Instrumental", SR);
  assert.deepEqual(first.slice(-3), [
    { kind: "SetMix", track: 0, params: { gain: 1, pan: 0, mode: "Soloed" } },
    { kind: "Seek", frame: 110 },
    { kind: "Play" },
  ]);
  s = run(s, { kind: "heard" });
  assert.deepEqual(observeListening(song, s, { kind: "Playing", frame: 200 }, SR), { kind: "ended" });
  const next = run(s, { kind: "ended" });
  assert.deepEqual(listenCommands(song, s, next, "Instrumental", SR).slice(-3), [
    { kind: "SetMix", track: 2, params: { gain: 1, pan: 0, mode: "Soloed" } },
    { kind: "Seek", frame: 110 },
    { kind: "Play" },
  ]);
});
