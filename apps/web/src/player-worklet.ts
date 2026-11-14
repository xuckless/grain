// AudioWorklet running engine-core's Player through engine-wasm (AGENTS.md §9).
import "./worklet-globals";
import { initSync, PlayerBuilder, type WasmPlayer } from "engine-wasm";
import { ringViews, writeBlock, type Ring } from "./capture-ring";
import type { FromWorklet, ToWorklet } from "./worklet-protocol";

declare class AudioWorkletProcessor {
  readonly port: MessagePort;
}
declare const sampleRate: number;
declare function registerProcessor(name: string, ctor: new () => AudioWorkletProcessor): void;

/** Post the transport every this many render quanta (~23 ms at 128 frames / 44.1 kHz). */
const TRANSPORT_EVERY = 8;

type Output = { player: WasmPlayer; memory: WebAssembly.Memory; buffer: ArrayBuffer; left: Float32Array; right: Float32Array; input: Float32Array };

type WorkletState =
  | { kind: "uninitialized" }
  | { kind: "idle"; memory: WebAssembly.Memory }
  | { kind: "playing"; output: Output };

/** Whether the microphone input is being written to a capture ring. */
type CaptureState = { kind: "off" } | { kind: "on"; ring: Ring };

class PlayerProcessor extends AudioWorkletProcessor {
  private state: WorkletState = { kind: "uninitialized" };
  private capture: CaptureState = { kind: "off" };
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
        builder.setReference(message.reference);
        const player = builder.build(sampleRate);
        this.state = { kind: "playing", output: this.views(player, memory) };
        this.post({ kind: "loaded", monitorLatency: player.monitorLatency() });
        this.post({ kind: "transport", transport: player.transport() });
        return;
      }
      case "addTrack": {
        if (this.state.kind !== "playing") return this.post({ kind: "rejected", message: "no song loaded" });
        try {
          this.state.output.player.addTrack(message.track.left, message.track.right, message.track.mix);
        } catch (e) {
          this.post({ kind: "rejected", message: e instanceof Error ? e.message : String(e) });
        }
        return;
      }
      case "capture": {
        if (this.state.kind !== "playing") return this.post({ kind: "rejected", message: "no song loaded" });
        try {
          this.state.output.player.apply(message.command);
        } catch (e) {
          return this.post({ kind: "rejected", message: e instanceof Error ? e.message : String(e) });
        }
        this.capture = { kind: "on", ring: ringViews(message.ring) };
        this.post({ kind: "cueStarted", transport: this.state.output.player.transport() });
        return;
      }
      case "endCapture":
        this.capture = { kind: "off" };
        return;
      case "unload": {
        if (this.state.kind !== "playing") return;
        const memory = this.state.output.memory;
        this.state.output.player.free();
        this.state = { kind: "idle", memory };
        this.capture = { kind: "off" };
        this.post({ kind: "transport", transport: { kind: "Stopped" } });
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

  /** Float32 views onto the player's output and input buffers in wasm memory. */
  private views(player: WasmPlayer, memory: WebAssembly.Memory): Output {
    const buffer = memory.buffer as ArrayBuffer;
    return {
      player,
      memory,
      buffer,
      left: new Float32Array(buffer, player.leftPtr(), 1024),
      right: new Float32Array(buffer, player.rightPtr(), 1024),
      input: new Float32Array(buffer, player.inputPtr(), 1024),
    };
  }

  /** The output views, renewed if wasm memory grew (and detached the old ones). */
  private fresh(out: Output): Output {
    if (out.memory.buffer === out.buffer) return out;
    const renewed = this.views(out.player, out.memory);
    this.state = { kind: "playing", output: renewed };
    return renewed;
  }

  process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    if (this.state.kind !== "playing") return true;
    let out = this.fresh(this.state.output);
    const [left, right] = outputs[0];
    // The microphone block of this quantum lines up with the output block rendered below.
    const mic = inputs[0]?.[0];
    if (this.capture.kind === "on") writeBlock(this.capture.ring, mic, left.length);
    // The monitor hears the microphone while one is connected.
    const withInput = mic !== undefined && mic.length === left.length;
    if (withInput) out.input.set(mic);
    out.player.render(left.length, withInput);
    out = this.fresh(out);
    left.set(out.left.subarray(0, left.length));
    right.set(out.right.subarray(0, right.length));
    if (++this.blocks % TRANSPORT_EVERY === 0) this.post({ kind: "transport", transport: out.player.transport() });
    return true;
  }
}

registerProcessor("player", PlayerProcessor);
