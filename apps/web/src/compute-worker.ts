// Off-main-thread engine work (AGENTS.md §10.1): engine-wasm calls that take longer than a
// frame's budget and need only audio in and a value out run here, so the UI keeps its frames.

import { ChainRenderer, chainPickAudio, initSync } from "engine-wasm";
import type { ToCompute, FromCompute } from "./compute-protocol";

let ready = false;
/** Keeps the per-step cache of the take last rendered (AGENTS.md §8.7). */
let chains: ChainRenderer | undefined;

self.onmessage = (e: MessageEvent<ToCompute>) => {
  const m = e.data;
  const reply = (r: FromCompute) => self.postMessage(r);
  switch (m.kind) {
    case "init":
      initSync({ module: m.module });
      ready = true;
      return;
    case "chainPick":
    case "chain":
      if (!ready) return reply({ kind: "failed", id: m.id, message: "compute worker not started" });
      try {
        if (m.kind === "chainPick") return reply({ kind: "done", id: m.id, value: chainPickAudio(m.audio, m.sampleRate, m.bpm, m.preset) });
        const out = (chains ??= new ChainRenderer()).render(m.project, m.take, m.audio);
        self.postMessage({ kind: "done", id: m.id, value: out } satisfies FromCompute, { transfer: [out.buffer] });
      } catch (err) {
        reply({ kind: "failed", id: m.id, message: err instanceof Error ? err.message : String(err) });
      }
      return;
  }
};
