// AudioWorklet running engine-core's Player through engine-wasm (AGENTS.md §9).
import "./worklet-globals";
import { initSync, PlayerBuilder, type WasmPlayer } from "engine-wasm";
import type { FromWorklet, ToWorklet } from "./worklet-protocol";

declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}
declare function registerProcessor(name: string, ctor: new () => AudioWorkletProcessor): void;

/** Post the transport every this many render quanta (~23 ms at 128 frames / 44.1 kHz). */
const TRANSPORT_EVERY = 8;

type Output = { player: WasmPlayer; memory: WebAssembly.Memory; buffer: ArrayBuffer; left: Float32Array; right: Float32Array };

type WorkletState =
  | { kind: "uninitialized" }
  | { kind: "idle"; memory: WebAssembly.Memory }
  | { kind: "playing"; output: Output };

class PlayerProcessor extends AudioWorkletProcessor {
  private state: WorkletState = { kind: "uninitialized" };
  private blocks = 0;

  constructor() {
    super();
    this.port.onmessage = (e: MessageEvent<ToWorklet>) => this.receive(e.data);
  }

  private post(message: FromWorklet) {
    this.port.postMessage(message);
  }

  private receive(message: ToWorklet) {
    switch (message.kind) {
      case "init": {
        const exports = initSync({ module: message.module });
        this.state = { kind: "idle", memory: exports.memory };
        this.post({ kind: "ready" });
        return;
      }
      case "load": {
        if (this.state.kind === "uninitialized") return this.post({ kind: "rejected", message: "worklet not initialized" });
        const memory = this.state.kind === "idle" ? this.state.memory : this.state.output.memory;
        if (this.state.kind === "playing") this.state.output.player.free();
        const builder = new PlayerBuilder();
        for (const t of message.tracks) builder.pushTrack(t.left, t.right, t.mix);
        const player = builder.build();
        this.state = { kind: "playing", output: this.views(player, memory) };
        this.post({ kind: "transport", transport: player.transport() });
        return;
      }
      case "command": {
        if (this.state.kind !== "playing") return this.post({ kind: "rejected", message: "no song loaded" });
        try {
          this.state.output.player.apply(message.command);
        } catch (e) {
          this.post({ kind: "rejected", message: e instanceof Error ? e.message : String(e) });
        }
        this.post({ kind: "transport", transport: this.state.output.player.transport() });
        return;
      }
    }
  }

  /** Float32 views onto the player's output buffers in wasm memory. */
  private views(player: WasmPlayer, memory: WebAssembly.Memory): Output {
    const buffer = memory.buffer as ArrayBuffer;
    return {
      player,
      memory,
      buffer,
      left: new Float32Array(buffer, player.leftPtr(), 1024),
      right: new Float32Array(buffer, player.rightPtr(), 1024),
    };
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    if (this.state.kind !== "playing") return true;
    let out = this.state.output;
    const [left, right] = outputs[0];
    out.player.render(left.length);
    // Wasm memory may have grown (and detached old views) since the last block.
    if (out.memory.buffer !== out.buffer) {
      out = this.views(out.player, out.memory);
      this.state = { kind: "playing", output: out };
    }
    left.set(out.left.subarray(0, left.length));
    right.set(out.right.subarray(0, right.length));
    if (++this.blocks % TRANSPORT_EVERY === 0) this.post({ kind: "transport", transport: out.player.transport() });
    return true;
  }
}

registerProcessor("player", PlayerProcessor);
