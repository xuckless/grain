// Web engine host: engine-wasm on the main thread, the player in an AudioWorklet,
// HTDemucs through onnxruntime-web when the weights are served at /models.

import type {
  EngineHost,
  ImportOutcome,
  Latency,
  Microphone,
  PlayerCommand,
  Project,
  ScoreOutcome,
  ScoreReport,
  ScoreSettings,
  SeparationQuality,
  StageListener,
  TakeAdded,
  Transport,
  TransportSnapshot,
} from "@workstation/engine-api";
import init, { countIn, measureLatency, WasmSeparator, WasmSong } from "engine-wasm";
import { createRing, drain, ringViews, type Ring } from "./capture-ring";
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

/** An open microphone feeding the player worklet's input. */
type OpenMic = { stream: MediaStream; source: MediaStreamAudioSourceNode; ring: Ring; buffer: SharedArrayBuffer };

/** What the microphone is doing. */
type MicState =
  | { kind: "closed" }
  | { kind: "recording"; mic: OpenMic; chunks: Float32Array[]; timer: number; fromSeconds: number; countInSeconds: number }
  | { kind: "calibrating" };

/** The browser's audio processing would change the singer's voice and fight calibration. */
const RAW_AUDIO: MediaTrackConstraints = { echoCancellation: false, noiseSuppression: false, autoGainControl: false };
/** Time for a just-opened microphone to settle before a cue starts. */
const MIC_SETTLE_MS = 300;
const DRAIN_MS = 50;
/** Longest echo a calibration waits for after the chirp (engine-core `PROBE_TAIL_SECONDS`). */
const PROBE_TAIL_MS = 1100;
const MAX_LATENCY_MS = 1000;
const LATENCY_KEY = "grain.latency.";

export async function createWebHost(): Promise<EngineHost> {
  const module = await WebAssembly.compileStreaming(fetch(wasmUrl));
  await init({ module_or_path: module });

  const context = new AudioContext({ sampleRate: SAMPLE_RATE, latencyHint: "interactive" });
  await context.audioWorklet.addModule(workletUrl);
  // Input 0 is the microphone, downmixed to mono, while a take or calibration records.
  const node = new AudioWorkletNode(context, "player", {
    numberOfInputs: 1,
    channelCount: 1,
    channelCountMode: "explicit",
    outputChannelCount: [2],
  });
  node.connect(context.destination);

  let snapshot: TransportSnapshot = { transport: { kind: "Stopped" }, sample_rate: context.sampleRate };
  let cueStarted: ((t: Transport) => void) | undefined;
  let rejected: ((message: string) => void) | undefined;
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
          rejected?.(m.message);
          return;
        case "cueStarted":
          snapshot = { transport: m.transport, sample_rate: context.sampleRate };
          cueStarted?.(m.transport);
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

  let mic: MicState = { kind: "closed" };

  function loadedSong(): WasmSong {
    if (session.kind !== "loaded") throw new Error("Load a song first.");
    return session.song;
  }

  async function openMic(): Promise<OpenMic> {
    await context.resume();
    const stream = await navigator.mediaDevices.getUserMedia({ audio: RAW_AUDIO });
    const source = context.createMediaStreamSource(stream);
    source.connect(node);
    await new Promise((r) => setTimeout(r, MIC_SETTLE_MS));
    const buffer = createRing();
    return { stream, source, buffer, ring: ringViews(buffer) };
  }

  function closeMic(m: OpenMic) {
    post({ kind: "endCapture" });
    m.source.disconnect();
    for (const t of m.stream.getTracks()) t.stop();
  }

  /** Starts capturing and applies `command` in the same quantum; resolves with the transport. */
  function startCue(m: OpenMic, command: PlayerCommand): Promise<Transport> {
    return new Promise((resolve, reject) => {
      cueStarted = (t) => {
        cueStarted = rejected = undefined;
        resolve(t);
      };
      rejected = (message) => {
        cueStarted = rejected = undefined;
        reject(new Error(message));
      };
      post({ kind: "capture", ring: m.buffer, command });
    });
  }

  async function devicePair(): Promise<string> {
    const inputs = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === "audioinput");
    const input = (inputs.find((d) => d.deviceId === "default") ?? inputs[0])?.label || "Microphone";
    const sink = (context as AudioContext & { sinkId?: string | { type: string } }).sinkId;
    const output = typeof sink === "string" && sink ? sink : "Default output";
    return `${input} | ${output}`;
  }

  async function storedLatency(): Promise<Latency> {
    const devices = await devicePair();
    try {
      const saved = localStorage.getItem(LATENCY_KEY + devices);
      if (saved) return JSON.parse(saved) as Latency;
    } catch {
      // Storage blocked or corrupt: use the browser's estimate.
    }
    const estimate = (context.baseLatency + (context.outputLatency || 0)) * 1000;
    return { ms: Math.round(estimate), source: { kind: "Default" }, devices };
  }

  function storeLatency(latency: Latency): Latency {
    try {
      localStorage.setItem(LATENCY_KEY + latency.devices, JSON.stringify(latency));
    } catch {
      // Not persisted (private window); still applies for this session.
    }
    return latency;
  }

  async function stopPlayback() {
    if (snapshot.transport.kind !== "Stopped") post({ kind: "command", command: { kind: "Stop" } });
  }

  /** Adds a scored take's audio to the player. */
  function playTake(song: WasmSong, added: TakeAdded): TakeAdded {
    const audio = song.takeAudio(added.take);
    const take = added.project.takes.find((t) => t.id === added.take)!;
    const right = new Float32Array(audio);
    post({ kind: "addTrack", track: { left: audio, right, mix: take.mix } }, [audio.buffer, right.buffer]);
    return added;
  }

  const microphone: Microphone =
    !crossOriginIsolated || !navigator.mediaDevices?.getUserMedia
      ? { kind: "unavailable", reason: "Recording needs a browser with microphone access and cross-origin isolation." }
      : {
          kind: "available",
          async start() {
            const song = loadedSong();
            if (mic.kind !== "closed") throw new Error("The microphone is busy.");
            mic = { kind: "calibrating" }; // Held while the microphone opens.
            try {
              const m = await openMic();
              const project = song.project() as Project;
              const command = countIn(project.analysis.tempo.bpm, context.sampleRate) as PlayerCommand & { kind: "CountIn" };
              const t = await startCue(m, command);
              const from = t.kind === "CountIn" ? t.frame : 0;
              const chunks: Float32Array[] = [];
              const timer = window.setInterval(() => chunks.push(drain(m.ring)), DRAIN_MS);
              mic = {
                kind: "recording",
                mic: m,
                chunks,
                timer,
                fromSeconds: from / context.sampleRate,
                countInSeconds: (command.beats * command.beat_frames) / context.sampleRate,
              };
            } catch (e) {
              mic = { kind: "closed" };
              throw e;
            }
          },
          async level() {
            return mic.kind === "recording" ? mic.mic.ring.level[0] : 0;
          },
          async finish(settings: ScoreSettings) {
            if (mic.kind !== "recording") throw new Error("No take is being recorded.");
            const r = mic;
            mic = { kind: "closed" };
            window.clearInterval(r.timer);
            await stopPlayback();
            closeMic(r.mic);
            r.chunks.push(drain(r.mic.ring));
            const capture = concat(r.chunks);
            const latency = await storedLatency();
            // The cue started at capture sample 0 (same render quantum).
            const songZero = r.countInSeconds + latency.ms / 1000 - r.fromSeconds;
            const song = loadedSong();
            const added = song.addRecordedTake(capture, context.sampleRate, songZero, latency.ms, settings) as TakeAdded;
            return playTake(song, added);
          },
          async cancel() {
            if (mic.kind !== "recording") return;
            window.clearInterval(mic.timer);
            closeMic(mic.mic);
            mic = { kind: "closed" };
            await stopPlayback();
          },
          latency: storedLatency,
          async setLatency(ms: number) {
            const devices = await devicePair();
            return storeLatency({ ms: Math.min(MAX_LATENCY_MS, Math.max(0, ms)), source: { kind: "Manual" }, devices });
          },
          async measureLatency() {
            loadedSong();
            if (mic.kind !== "closed") throw new Error("The microphone is busy.");
            mic = { kind: "calibrating" };
            try {
              if (snapshot.transport.kind !== "Stopped" && snapshot.transport.kind !== "Paused") {
                post({ kind: "command", command: { kind: "Stop" } });
              }
              const m = await openMic();
              try {
                await startCue(m, { kind: "Probe" });
                while (snapshot.transport.kind === "Probing") await new Promise((r) => setTimeout(r, 20));
                await new Promise((r) => setTimeout(r, PROBE_TAIL_MS));
              } finally {
                closeMic(m);
              }
              const [ms, confidence] = measureLatency(drain(m.ring), context.sampleRate, 0);
              return storeLatency({ ms, source: { kind: "Measured", confidence }, devices: await devicePair() });
            } finally {
              mic = { kind: "closed" };
            }
          },
        };

  // The cascade's vocal model is ~930 MB and peaks near 8 GB of memory: desktop only.
  async function importFile(_quality: SeparationQuality, onStage: StageListener): Promise<ImportOutcome> {
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

  async function scoreTake(settings: ScoreSettings): Promise<ScoreOutcome> {
    const song = loadedSong();
    const file = await pickFile("audio/*");
    if (!file) return { kind: "cancelled" };
    const [left, right] = await decode(context, file);
    const added = song.addFileTake(file.name, left, right, settings) as TakeAdded;
    return { kind: "scored", added: playTake(song, added) };
  }

  return {
    chrome: { kind: "plain" },
    projects: { kind: "unavailable", reason: "Saving and recent songs need the desktop app for now." },
    async close() {
      if (microphone.kind === "available") await microphone.cancel();
      post({ kind: "unload" });
      if (session.kind === "loaded") session.song.free();
      session = { kind: "empty" };
    },
    urlImport: { kind: "unavailable" },
    separation: { kind: "fastOnly", reason: "Best vocals needs the desktop app (a 930 MB model and about 8 GB of memory)." },
    importFile,
    scoreTake,
    async scoreRecorded(id, settings) {
      return loadedSong().scoreTake(id, settings) as ScoreReport;
    },
    microphone,
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

function concat(chunks: Float32Array[]): Float32Array {
  const out = new Float32Array(chunks.reduce((n, c) => n + c.length, 0));
  let at = 0;
  for (const c of chunks) {
    out.set(c, at);
    at += c.length;
  }
  return out;
}
