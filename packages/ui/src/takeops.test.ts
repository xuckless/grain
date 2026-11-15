// Run with `pnpm --filter @workstation/ui test`.
import assert from "node:assert/strict";
import { test } from "node:test";
import type { Latency, Project, Take, TakeStore } from "@workstation/engine-api";
import { masterId } from "./chain.ts";
import { emptyHistory, record, redo, undo, type Command, type History } from "./history.ts";
import { calibrateBeforeRecording, calibrationNote } from "./takes.ts";
import { applyTakeCommand, deleteTake, replaceTake, type TakeSession } from "./takeops.ts";
import type { ScoreState } from "./state.ts";

const sung = (id: number, name = `Take ${id}`) => ({ id, name, source: { kind: "Recorded", latency_ms: 0 } }) as unknown as Take;

/** A take store with the hosts' set-aside rules (engine-core `take::set_aside` and friends). */
function fakeStore(initial: Project): TakeStore & { project: Project } {
  type Aside = { take: Take; index: number; wasMaster: boolean };
  const aside: Aside[] = [];
  const store = {
    project: initial,
    setAside(id: number) {
      const p = store.project;
      const index = p.takes.findIndex((t) => t.id === id);
      if (index < 0) throw new Error(`no take ${id}`);
      aside.push({ take: p.takes[index], index, wasMaster: p.master_take === id });
      store.project = { ...p, takes: p.takes.filter((t) => t.id !== id), master_take: p.master_take === id ? null : p.master_take };
      return aside.at(-1)!;
    },
    putBack(id: number) {
      const k = aside.findIndex((a) => a.take.id === id);
      if (k < 0) throw new Error(`no take ${id} set aside`);
      const [a] = aside.splice(k, 1);
      const takes = [...store.project.takes];
      takes.splice(a.index, 0, a.take);
      store.project = { ...store.project, takes, master_take: a.wasMaster ? id : store.project.master_take };
    },
    async remove(id: number) {
      store.setAside(id);
      return store.project;
    },
    async restore(id: number) {
      store.putBack(id);
      return store.project;
    },
    async replace(target: number, replacement: number) {
      if (aside.some((a) => a.take.id === replacement)) {
        store.setAside(target);
        store.putBack(replacement);
        return store.project;
      }
      const t = store.project.takes.find((x) => x.id === replacement);
      if (!t || target === replacement) throw new Error(`no take ${replacement}`);
      const out = store.setAside(target);
      const takes = store.project.takes.filter((x) => x.id !== replacement);
      takes.splice(out.index, 0, { ...t, name: out.take.name });
      store.project = { ...store.project, takes, master_take: out.wasMaster ? replacement : store.project.master_take };
      return store.project;
    },
    rename: async () => store.project,
    setChain: async () => ({ kind: "Rendered", project: store.project }) as const,
    setMaster: async () => store.project,
  };
  return store;
}

function session(project: Project): TakeSession & { history: History } {
  const s = {
    project,
    take: null as number | null,
    score: { kind: "none" } as ScoreState,
    history: emptyHistory,
    saved(p: Project) {
      s.project = p;
    },
    record(c: Command) {
      s.history = record(s.history, c);
    },
  };
  return s;
}

const ids = (p: Project) => p.takes.map((t) => t.id);

test("deleting the ★ master falls back to the newest take, and ⌘Z restores both", async () => {
  const store = fakeStore({ takes: [sung(1), sung(2), sung(3)], master_take: 2 } as unknown as Project);
  const s = session(store.project);
  s.take = 2;
  await deleteTake(store, s, 2);
  assert.deepEqual(ids(s.project), [1, 3]);
  assert.equal(masterId(s.project), 3, "the newest remaining take is the master");
  assert.equal(s.take, null, "views fall back to the master");
  const u = undo(s.history)!;
  await applyTakeCommand(store, s, u.command as never, "undo");
  assert.deepEqual(ids(s.project), [1, 2, 3], "back in its slot");
  assert.equal(masterId(s.project), 2, "and starred again");
  const r = redo(u.history)!;
  await applyTakeCommand(store, s, r.command as never, "redo");
  assert.deepEqual(ids(s.project), [1, 3]);
});

test("a saved retake replaces its take in place (name, slot, ★) as one undoable step", async () => {
  const store = fakeStore({ takes: [sung(1, "Verse"), sung(2), sung(4)], master_take: 1 } as unknown as Project);
  const s = session(store.project);
  s.take = 1;
  // Take 4 is the retake of take 1, just saved as a new take.
  await replaceTake(store, s, 1, 4);
  assert.deepEqual(ids(s.project), [4, 2]);
  assert.equal(s.project.takes[0].name, "Verse");
  assert.equal(masterId(s.project), 4, "the ★ moved over");
  assert.equal(s.take, 4, "the views follow the slot");
  assert.equal(s.history.past.length, 1);
  const u = undo(s.history)!;
  await applyTakeCommand(store, s, u.command as never, "undo");
  assert.deepEqual(ids(s.project), [1, 2]);
  assert.equal(masterId(s.project), 1);
  assert.equal(s.take, 1);
  await applyTakeCommand(store, s, redo(u.history)!.command as never, "redo");
  assert.deepEqual(ids(s.project), [4, 2]);
  assert.equal(masterId(s.project), 4);
});

test("a retake that fails to save never touches the original", async () => {
  const store = fakeStore({ takes: [sung(1), sung(2)], master_take: 1 } as unknown as Project);
  const s = session(store.project);
  // Cancelling, or a failed save, never calls `replaceTake`; a failed swap leaves both.
  await assert.rejects(replaceTake(store, s, 1, 9));
  assert.deepEqual(ids(s.project), [1, 2]);
  assert.equal(masterId(s.project), 1);
  assert.equal(s.history.past.length, 0);
});

const latency = (source: Latency["source"]): Latency => ({ ms: 120, source, devices: "mic | out" });

test("auto-calibration: a heard chirp is measured; an unheard one keeps the stored latency; both go on to record", async () => {
  const measured = latency({ kind: "Measured", confidence: 0.8 });
  const heard = await calibrateBeforeRecording({ measureLatency: async () => measured, latency: async () => latency({ kind: "Manual" }) });
  assert.deepEqual(heard, { kind: "ready", latency: measured, source: "Measured" });
  assert.equal(calibrationNote(heard), null);
  const notHeard = () => Promise.reject(new Error("the chirp wasn't heard"));
  const previous = await calibrateBeforeRecording({ measureLatency: notHeard, latency: async () => latency({ kind: "Measured", confidence: 0.5 }) });
  assert.equal(previous.kind, "ready");
  assert.equal(previous.source, "Previous");
  assert.match(calibrationNote(previous)!, /last latency \(120 ms\)/);
  const estimated = await calibrateBeforeRecording({ measureLatency: notHeard, latency: async () => latency({ kind: "Default" }) });
  assert.equal(estimated.source, "Estimated");
});
