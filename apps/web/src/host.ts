// Web engine host: engine-wasm on the main thread, the player in an AudioWorklet,
// HTDemucs through onnxruntime-web when the weights are served at /models.

import type { EngineHost, ImportOutcome, Project, ScoreOutcome, StageListener, TransportSnapshot } from "@workstation/engine-api";
import init, { WasmSeparator, WasmSong } from "engine-wasm";
import wasmUrl from "engine-wasm/engine_wasm_bg.wasm?url";
import workletUrl from "./player-worklet.ts?worker&url";
import type { FromWorklet, ToWorklet, TrackData } from "./worklet-protocol";

/** Separation and analysis run at the engine's working rate; the AudioContext matches it. */
const SAMPLE_RATE = 44_100;
const MODEL = "htdemucs_6s";

type Separation =
  | { kind: "available"; spec: string; run: (mix: Float32Array, mag: Float32Array) => Promise<[Float32Array, Float32Array]> }
  | { kind: "unavailable" };

type Session = { kind: "empty" } | { kind: "loaded"; song: WasmSong };

export async function createWebHost(): Promise<EngineHost> {
  const module = await WebAssembly.compileStreaming(fetch(wasmUrl));
  await init({ module_or_path: module });

  const context = new AudioContext({ sampleRate: SAMPLE_RATE, latencyHint: "interactive" });
  await context.audioWorklet.addModule(workletUrl);
  const node = new AudioWorkletNode(context, "player", { numberOfInputs: 0, outputChannelCount: [2] });
  node.connect(context.destination);

  let snapshot: TransportSnapshot = { transport: { kind: "Stopped" }, sample_rate: context.sampleRate };
  const ready = new Promise<void>((resolve) => {
    node.port.onmessage = (e: MessageEvent<FromWorklet>) => {
      const m = e.data;
      switch (m.kind) {
        case "ready":
          resolve();
          return;
        case "transport":
          snapshot = { transport: m.transport, sample_rate: context.sampleRate };
          return;
        case "rejected":
          console.warn(`player: ${m.message}`);
          return;
      }
    };
  });
  const post = (message: ToWorklet, transfer: Transferable[] = []) => node.port.postMessage(message, transfer);
  post({ kind: "init", module });
  await ready;

  let session: Session = { kind: "empty" };
  let separation: Promise<Separation> | undefined;

  function load(song: WasmSong): Project {
    const project = song.project() as Project;
    const tracks: TrackData[] = project.tracks.map((t, i) => {
      const [left, right] = song.trackAudio(i) as [Float32Array, Float32Array];
      return { left, right, mix: t.mix };
    });
    post({ kind: "load", tracks }, tracks.flatMap((t) => [t.left.buffer, t.right.buffer]));
    if (session.kind === "loaded") session.song.free();
    session = { kind: "loaded", song };
    return project;
  }

  async function importFile(onStage: StageListener): Promise<ImportOutcome> {
    // Open the picker synchronously: browsers (Safari especially) drop the click's user
    // activation across an await, which silently blocks the file dialog.
    const picked = pickFile("audio/*");
    void context.resume();
    const file = await picked;
    if (!file) return { kind: "cancelled" };
    onStage({ kind: "Decoding" });
    const [left, right] = await decode(context, file);
    const sep = await (separation ??= loadSeparation());
    switch (sep.kind) {
      case "unavailable":
        onStage({ kind: "Analyzing" });
        await nextFrame();
        return { kind: "imported", project: load(WasmSong.unseparated(file.name, left, right, SAMPLE_RATE)) };
      case "available": {
        const separator = new WasmSeparator(sep.spec, file.name, left, right);
        for (;;) {
          const state = separator.state() as { kind: "AwaitingSegment"; index: number; count: number } | { kind: "Complete" };
          if (state.kind === "Complete") break;
          onStage({ kind: "Separating", segment: state.index, of: state.count });
          const [mix, mag] = separator.inputs() as [Float32Array, Float32Array];
          const [spec, wav] = await sep.run(mix, mag);
          separator.submit(spec, wav);
        }
        onStage({ kind: "Analyzing" });
        await nextFrame();
        return { kind: "imported", project: load(separator.intoSong()) };
      }
    }
  }

  async function scoreTake(settings: import("@workstation/engine-api").ScoreSettings): Promise<ScoreOutcome> {
    if (session.kind !== "loaded") throw new Error("Load a song first.");
    const file = await pickFile("audio/*");
    if (!file) return { kind: "cancelled" };
    const [left, right] = await decode(context, file);
    return { kind: "scored", report: session.song.scoreTake(left, right, settings) };
  }

  return {
    chrome: { kind: "plain" },
    projects: { kind: "unavailable", reason: "Saving and recent songs need the desktop app for now." },
    async close() {
      post({ kind: "unload" });
      if (session.kind === "loaded") session.song.free();
      session = { kind: "empty" };
    },
    urlImport: { kind: "unavailable" },
    importFile,
    scoreTake,
    async send(command) {
      await context.resume();
      post({ kind: "command", command });
    },
    async transport() {
      return snapshot;
    },
  };
}

/** HTDemucs through onnxruntime-web (WebGPU, falling back to WASM) if /models serves it. */
async function loadSeparation(): Promise<Separation> {
  const specResponse = await fetch(`/models/${MODEL}.json`);
  if (!specResponse.ok) return { kind: "unavailable" };
  const spec = await specResponse.text();
  const ort = await import("onnxruntime-web/webgpu");
  const session = await ort.InferenceSession.create(`/models/${MODEL}.onnx`, {
    executionProviders: ["webgpu", "wasm"],
  });
  const { segment_samples: s, freq_bins: f, frames: t } = JSON.parse(spec) as { segment_samples: number; freq_bins: number; frames: number };
  return {
    kind: "available",
    spec,
    async run(mix, mag) {
      const out = await session.run({
        mix: new ort.Tensor("float32", mix, [1, 2, s]),
        mag: new ort.Tensor("float32", mag, [1, 4, f, t]),
      });
      return [(await out.spec.getData()) as Float32Array, (await out.wav.getData()) as Float32Array];
    },
  };
}

async function decode(context: AudioContext, file: File): Promise<[Float32Array, Float32Array]> {
  const audio = await context.decodeAudioData(await file.arrayBuffer());
  const left = audio.getChannelData(0);
  const right = audio.numberOfChannels > 1 ? audio.getChannelData(1) : left;
  return [new Float32Array(left), new Float32Array(right)];
}

function pickFile(accept: string): Promise<File | undefined> {
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept;
    input.onchange = () => resolve(input.files?.[0]);
    input.oncancel = () => resolve(undefined);
    // Safari only opens pickers for inputs attached to the document.
    input.hidden = true;
    document.body.append(input);
    const done = () => input.remove();
    input.addEventListener("change", done, { once: true });
    input.addEventListener("cancel", done, { once: true });
    input.click();
  });
}

/** Lets the UI paint the stage before a long synchronous wasm call. */
function nextFrame(): Promise<void> {
  return new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
}
