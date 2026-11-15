import assert from "node:assert/strict";
import test from "node:test";
import type { Project } from "@workstation/engine-api";
import { exportNext, exportTracks, type ExportState } from "./exporting.ts";

const mix = { gain: 1, pan: 0, mode: "Normal" } as const;
const muted = { ...mix, mode: "Muted" } as const;
const song = {
  master_take: 1,
  tracks: [
    { name: "Vocals", source: { kind: "Stem", stem: "Vocals" }, mix },
    { name: "Drums", source: { kind: "Stem", stem: "Drums" }, mix },
    { name: "Bass", source: { kind: "Stem", stem: "Bass" }, mix: muted },
  ],
  takes: [
    { id: 1, name: "Take 1", mix: muted, source: { kind: "Recorded" } },
    { id: 2, name: "Take 2", mix: muted, source: { kind: "Recorded" } },
    { id: 3, name: "Harmony", mix, source: { kind: "Harmony" } },
  ],
} as unknown as Project;

test("export never offers the guide vocal and ticks the master take with the audible mix", () => {
  const rows = exportTracks(song);
  assert.ok(!rows.some((r) => r.index === 0), "the vocal stem is never exported");
  assert.deepEqual(
    rows.filter((r) => r.checked).map((r) => r.index),
    [1, 3, 5],
  );
  assert.equal(rows.find((r) => r.index === 3)?.label, "★ Take 1");
});

test("solo on a song track leaves the others out by default", () => {
  const solo = { ...song, tracks: song.tracks.map((t, i) => (i === 2 ? { ...t, mix: { ...mix, mode: "Soloed" } } : t)) } as Project;
  assert.deepEqual(
    exportTracks(solo).filter((r) => r.checked).map((r) => r.index),
    [2, 3],
  );
});

test("the export dialog runs one export at a time through explicit states", () => {
  let s: ExportState = { kind: "idle" };
  const step = (e: Parameters<typeof exportNext>[1]) => (s = exportNext(s, e) ?? assert.fail(`${e.kind} rejected in ${s.kind}`));
  step({ kind: "start" });
  assert.equal(exportNext(s, { kind: "start" }), null);
  step({ kind: "progress", fraction: 0.5 });
  step({ kind: "encoding" });
  step({ kind: "saved", path: "/x.wav" });
  assert.deepEqual(s, { kind: "done", path: "/x.wav" });
  step({ kind: "start" });
  step({ kind: "fail", message: "disk full" });
  assert.equal(s.kind, "failed");
});
