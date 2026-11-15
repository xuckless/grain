import assert from "node:assert/strict";
import test from "node:test";
import type { Project, TakeChain } from "@workstation/engine-api";
import { masterId, selectedTakeId, withActive, withBypass, withUnset } from "./chain.ts";

const unset: TakeChain = { pitch: { state: "Unset" }, tune: { state: "Unset" }, clean: { state: "Unset" }, vocal: { state: "Unset" } };
const take = (id: number, kind = "Recorded") => ({ id, source: { kind }, chain: unset });
const project = (master: number | null) =>
  ({ master_take: master, takes: [take(1), take(2), take(3, "Harmony")] }) as unknown as Project;

test("the master is the starred sung take, else the newest sung take", () => {
  assert.equal(masterId(project(null)), 2);
  assert.equal(masterId(project(1)), 1);
  assert.equal(masterId(project(3)), 2); // a harmony track can't be the master
  assert.equal(masterId(project(9)), 2); // a deleted star falls back
});

test("the selected take defaults to the master and follows a pick while it exists", () => {
  assert.equal(selectedTakeId(project(1), null), 1);
  assert.equal(selectedTakeId(project(1), 2), 2);
  assert.equal(selectedTakeId(project(1), 7), 1);
});

test("bypass keeps a step's parameters; unset clears it", () => {
  const on = withActive(unset, "clean", { model: "dfn" });
  const off = withBypass(on, "clean", true);
  assert.deepEqual(off.clean, { state: "Bypassed", params: { model: "dfn" } });
  assert.deepEqual(withBypass(off, "clean", false), on);
  assert.deepEqual(withBypass(unset, "tune", true), unset);
  assert.deepEqual(withUnset(on, "clean"), unset);
});
