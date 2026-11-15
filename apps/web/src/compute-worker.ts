// Off-main-thread engine work (AGENTS.md §10.1): engine-wasm calls that take longer than a
// frame's budget and need only audio in and a value out run here, so the UI keeps its frames.

import { chainPickAudio, initSync } from "engine-wasm";
import type { ToCompute, FromCompute } from "./compute-protocol";

let ready = false;

self.onmessage = (e: MessageEvent<ToCompute>) => {
  const m = e.data;
  const reply = (r: FromCompute) => self.postMessage(r);
  switch (m.kind) {
    case "init":
      initSync({ module: m.module });
      ready = true;
      return;
    case "chainPick":
      if (!ready) return reply({ kind: "failed", id: m.id, message: "compute worker not started" });
      try {
        reply({ kind: "done", id: m.id, value: chainPickAudio(m.audio, m.sampleRate, m.bpm, m.preset) });
      } catch (err) {
        reply({ kind: "failed", id: m.id, message: err instanceof Error ? err.message : String(err) });
      }
      return;
  }
};
