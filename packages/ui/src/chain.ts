// The master take and its processing chain, UI side (AGENTS.md §8.7). Pitch, Auto-tune and
// Produce each edit their own step of the selected take; this module holds the pure helpers and
// the one way a step change reaches the host (and the undo history).

import type { CleanStep, EngineHost, PitchStep, Project, Step, Take, TakeChain, TuneSettings, VocalStep } from "@workstation/engine-api";
import { sungTakes } from "./listen.ts";

/** The ★ master: the starred sung take, or the newest (mirrors engine-core `process::master`). */
export function masterId(project: Project): number | null {
  const sung = sungTakes(project);
  if (project.master_take !== null && sung.some((t) => t.id === project.master_take)) return project.master_take;
  return sung.at(-1)?.id ?? null;
}

/** The take the views work on: the picked one while it exists, else the master. */
export function selectedTakeId(project: Project, picked: number | null): number | null {
  if (picked !== null && sungTakes(project).some((t) => t.id === picked)) return picked;
  return masterId(project);
}

export const takeById = (project: Project, id: number | null): Take | null => (id === null ? null : (project.takes.find((t) => t.id === id) ?? null));

export type StepKey = keyof TakeChain;

/** Each step's parameters. */
export type StepParams = { pitch: PitchStep; tune: TuneSettings; clean: CleanStep; vocal: VocalStep };

/** The step's parameters, whether on or bypassed. */
export function stepParams<T>(s: Step<T>): T | null {
  return s.state === "Unset" ? null : s.params;
}

/** The chain with step `key` on with `params`. */
export function withActive<K extends StepKey>(chain: TakeChain, key: K, params: StepParams[K]): TakeChain {
  return { ...chain, [key]: { state: "Active", params } };
}

/** The chain with step `key` switched on or bypassed (keeping its parameters); unset stays. */
export function withBypass(chain: TakeChain, key: StepKey, bypassed: boolean): TakeChain {
  const s = chain[key];
  if (s.state === "Unset") return chain;
  return { ...chain, [key]: { state: bypassed ? "Bypassed" : "Active", params: s.params } };
}

/** The chain with step `key` removed. */
export const withUnset = (chain: TakeChain, key: StepKey): TakeChain => ({ ...chain, [key]: { state: "Unset" } });

export const STEP_LABEL: Record<StepKey, string> = { pitch: "Pitch edits", tune: "Auto-tune", clean: "Clean up", vocal: "Vocal chain" };

/** A step change in flight (one at a time per session; ground rule 1). */
export type ChainJob =
  | { kind: "idle" }
  | { kind: "rendering"; take: number; step: StepKey }
  | { kind: "needsModel"; model: string }
  | { kind: "failed"; message: string };

/** What the session needs from a chain change: the saved project and the undo entry. */
export type ChainSink = {
  saved(project: Project): void;
  record(c: { kind: "Chain"; take: number; before: TakeChain; after: TakeChain; label: string }): void;
};

/** Sets take `take`'s chain to `after` on the host, renders it, and records the change for undo.
 * `record: false` for undo/redo themselves. */
export async function commitChain(
  host: EngineHost,
  sink: ChainSink,
  project: Project,
  take: number,
  after: TakeChain,
  step: StepKey,
  recordIt = true,
): Promise<ChainJob> {
  const before = takeById(project, take)?.chain;
  if (!before) return { kind: "failed", message: "That take was deleted." };
  try {
    const outcome = await host.takes.setChain(take, after);
    if (outcome.kind === "NeedsModel") return { kind: "needsModel", model: outcome.model };
    sink.saved(outcome.project);
    if (recordIt) sink.record({ kind: "Chain", take, before, after, label: STEP_LABEL[step] });
    return { kind: "idle" };
  } catch (e) {
    return { kind: "failed", message: e instanceof Error ? e.message : String(e) };
  }
}
