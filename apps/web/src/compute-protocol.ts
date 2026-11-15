// Messages between the main thread and the compute worker (compute-worker.ts).

import type { ChainPreset, Project } from "@workstation/engine-api";

/** Requests that carry audio (transferred, not copied). */
export type ComputeRequest =
  | { kind: "chainPick"; audio: Float32Array; sampleRate: number; bpm: number; preset: ChainPreset }
  /** Take `take` of `project` through its processing chain, from its raw `audio` (AGENTS.md §8.7). */
  | { kind: "chain"; audio: Float32Array; project: Project; take: number };

export type ToCompute = { kind: "init"; module: WebAssembly.Module } | (ComputeRequest & { id: number });

export type FromCompute = { kind: "done"; id: number; value: unknown } | { kind: "failed"; id: number; message: string };

/** The main thread's side: one worker, requests matched to replies by id. */
export class Compute {
  #worker: Worker;
  #next = 1;
  #pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();

  constructor(worker: Worker, module: WebAssembly.Module) {
    this.#worker = worker;
    worker.onmessage = (e: MessageEvent<FromCompute>) => {
      const m = e.data;
      const p = this.#pending.get(m.id);
      if (!p) return;
      this.#pending.delete(m.id);
      if (m.kind === "done") p.resolve(m.value);
      else p.reject(new Error(m.message));
    };
    this.#post({ kind: "init", module });
  }

  #post(m: ToCompute, transfer: Transferable[] = []) {
    this.#worker.postMessage(m, transfer);
  }

  /** Runs a request; its audio buffer is transferred (not copied). */
  run<T>(request: ComputeRequest): Promise<T> {
    const id = this.#next++;
    return new Promise<T>((resolve, reject) => {
      this.#pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.#post({ ...request, id }, [request.audio.buffer]);
    });
  }
}
