// Web engine host: engine-wasm on the main thread, the player in an AudioWorklet,
// models (HTDemucs, basic-pitch) through onnxruntime-web from the model store (./models).

import type {
  EngineHost,
  ImportOutcome,
  Latency,
  LiveUpdate,
  Microphone,
  MonitorMode,
  MonitorStatus,
  PlayerCommand,
  Project,
  ProjectLyrics,
  ProjectStore,
  ProjectSummary,
  ScoreOutcome,
  ScoreReport,
  ScoreSettings,
  SeparationQuality,
  StageListener,
  TakeAdded,
  Transport,
  TransportSnapshot,
  TunePreview,
  EditUpdate,
  PitchEditView,
  ChainApplied,
  ChainPick,
  GenerationRequest,
  Generator,
  GeneratorState,
} from "@workstation/engine-api";
import { melodyFileName } from "@workstation/engine-api";
import init, { basicPitchSpec, countIn, generationQuery, measureLatency, monitorStatus, projectSummary, transcriptionMethod, WasmLive, WasmSeparator, WasmSong } from "engine-wasm";
import { WebModels } from "./models";
import { createRing, drain, ringViews, type Ring } from "./capture-ring";
import * as store from "./store";
import wasmUrl from "engine-wasm/engine_wasm_bg.wasm?url";
import workletUrl from "./player-worklet.ts?worker&url";
import ComputeWorker from "./compute-worker.ts?worker";
import { Compute } from "./compute-protocol";
import type { FromWorklet, ToWorklet, TrackData } from "./worklet-protocol";

/** Separation and analysis run at the engine's working rate; the AudioContext matches it. */
const SAMPLE_RATE = 44_100;
const MODEL = "htdemucs_6s";

type Separation =
  | { kind: "available"; spec: string; run: (mix: Float32Array, mag: Float32Array) => Promise<[Float32Array, Float32Array]> }
  | { kind: "unavailable" };

/** `id`: the song's OPFS project, once saved or opened. */
type Session = { kind: "empty" } | { kind: "loaded"; song: WasmSong; id: string | null };

/** The microphone connected to the player worklet's input. Takes, calibration and the live
 * monitor share it (each is a holder), so the input never carries the voice twice. */
type MicInput =
  | { kind: "closed" }
  | { kind: "opening"; ready: Promise<void>; holders: number }
  | { kind: "open"; stream: MediaStream; source: MediaStreamAudioSourceNode; holders: number };

/** A capture of the microphone (a take or a calibration) into a ring. */
type OpenMic = { ring: Ring; buffer: SharedArrayBuffer };

/** Whether the live monitor holds the microphone input (the player knows dry or tuned). */
type MonitorHold = { kind: "off" } | { kind: "listening" };

/** What the microphone is doing. */
type MicState =
  | { kind: "closed" }
  | {
      kind: "recording";
      mic: OpenMic;
      chunks: Float32Array[];
      timer: number;
      fromSeconds: number;
      countInSeconds: number;
      /** Live karaoke pitch, started by the first `live()` call; `fed`: chunks already pushed. */
      live: { kind: "idle" } | { kind: "running"; session: WasmLive; fed: number };
    }
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

/** A live update with nothing sung yet. */
function noLive(): LiveUpdate {
  return { frames: [], totals: { hit: 0, miss: 0, reference_only: 0, user_only: 0 } };
}

export async function createWebHost(): Promise<EngineHost> {
  const module = await WebAssembly.compileStreaming(fetch(wasmUrl));
  await init({ module_or_path: module });
  const compute = new Compute(new ComputeWorker(), module);

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
  const transportListeners = new Set<(s: TransportSnapshot) => void>();
  const setSnapshot = (t: Transport) => {
    snapshot = { transport: t, sample_rate: context.sampleRate };
    for (const l of transportListeners) l(snapshot);
  };
  /** The tuned monitor's delay (frames), reported when a song loads. */
  let monitorFrames = 0;
  let cueStarted: ((t: Transport) => void) | undefined;
  let rejected: ((message: string) => void) | undefined;
  const ready = new Promise<void>((resolve) => {
    node.port.onmessage = (e: MessageEvent<FromWorklet>) => {
      const m = e.data;
      switch (m.kind) {
        case "ready":
          resolve();
          return;
        case "loaded":
          monitorFrames = m.monitorLatency;
          return;
        case "transport":
          setSnapshot(m.transport);
          return;
        case "rejected":
          console.warn(`player: ${m.message}`);
          rejected?.(m.message);
          return;
        case "cueStarted":
          setSnapshot(m.transport);
          cueStarted?.(m.transport);
          return;
      }
    };
  });
  const post = (message: ToWorklet, transfer: Transferable[] = []) => node.port.postMessage(message, transfer);
  post({ kind: "init", module });
  await ready;

  let session: Session = { kind: "empty" };
  const models = new WebModels();
  let separation: Promise<Separation> | undefined;
  let basicPitch: Promise<BasicPitch> | undefined;
  /** Installed models changed: load them afresh when next needed. */
  const modelsChanged = () => {
    separation = undefined;
    basicPitch = undefined;
  };

  /** Makes `song` the loaded song: its tracks, then its takes, go to the player. */
  function load(song: WasmSong, id: string | null = null): Project {
    const project = toPlayer(song);
    if (session.kind === "loaded" && session.song !== song) session.song.free();
    session = { kind: "loaded", song, id };
    return project;
  }

  /** Hands every track and take of `song` to a fresh player (its monitor starts off). */
  function toPlayer(song: WasmSong): Project {
    const project = song.project() as Project;
    const tracks: TrackData[] = [
      ...project.tracks.map((t, i) => {
        const [left, right] = song.trackAudio(i) as [Float32Array, Float32Array];
        return { left, right, mix: t.mix };
      }),
      ...project.takes.map((t) => {
        const left = song.takeAudio(t.id);
        return { left, right: new Float32Array(left), mix: t.mix };
      }),
    ];
    post({ kind: "load", tracks, reference: project.analysis.reference_notes }, tracks.flatMap((t) => [t.left.buffer, t.right.buffer]));
    endMonitor();
    return project;
  }

  let mic: MicState = { kind: "closed" };

  function loadedSong(): WasmSong {
    if (session.kind !== "loaded") throw new Error("Load a song first.");
    return session.song;
  }

  let input: MicInput = { kind: "closed" };

  /** Holds the microphone input, connecting it on the first hold. */
  async function holdInput(): Promise<void> {
    switch (input.kind) {
      case "open":
        input.holders += 1;
        return;
      case "opening":
        input.holders += 1;
        return input.ready;
      case "closed": {
        const opening = (async () => {
          await context.resume();
          const stream = await navigator.mediaDevices.getUserMedia({ audio: RAW_AUDIO });
          const source = context.createMediaStreamSource(stream);
          source.connect(node);
          opened(stream, source);
        })();
        input = { kind: "opening", ready: opening, holders: 1 };
        try {
          await opening;
        } catch (e) {
          input = { kind: "closed" };
          throw e;
        }
        return;
      }
    }
  }

  /** The microphone finished opening; if every holder let go meanwhile, it closes again. */
  function opened(stream: MediaStream, source: MediaStreamAudioSourceNode) {
    if (input.kind !== "opening") return;
    const open = { kind: "open", stream, source, holders: input.holders } as const;
    input = open;
    if (open.holders === 0) disconnect(open);
  }

  /** Lets go of the microphone input, disconnecting it with the last hold. */
  function releaseInput() {
    switch (input.kind) {
      case "closed":
        return;
      case "opening":
        input.holders -= 1;
        return;
      case "open":
        if (--input.holders === 0) disconnect(input);
        return;
    }
  }

  function disconnect(open: Extract<MicInput, { kind: "open" }>) {
    open.source.disconnect();
    for (const t of open.stream.getTracks()) t.stop();
    input = { kind: "closed" };
  }

  async function openMic(): Promise<OpenMic> {
    await holdInput();
    await new Promise((r) => setTimeout(r, MIC_SETTLE_MS));
    const buffer = createRing();
    return { buffer, ring: ringViews(buffer) };
  }

  function closeMic() {
    post({ kind: "endCapture" });
    releaseInput();
  }

  let monitor: MonitorHold = { kind: "off" };

  function endMonitor() {
    if (monitor.kind === "off") return;
    monitor = { kind: "off" };
    releaseInput();
  }

  async function setMonitor(mode: MonitorMode): Promise<MonitorStatus> {
    loadedSong();
    if (mode === "Off") {
      post({ kind: "command", command: { kind: "Monitor", mode } });
      endMonitor();
    } else {
      if (!crossOriginIsolated || !navigator.mediaDevices?.getUserMedia) throw new Error("Monitoring needs a browser with microphone access.");
      if (monitor.kind === "off") {
        await holdInput();
        monitor = { kind: "listening" };
      }
      post({ kind: "command", command: { kind: "Monitor", mode } });
    }
    const device = await storedLatency();
    return monitorStatus(mode, (monitorFrames * 1000) / context.sampleRate, device.ms) as MonitorStatus;
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

  /** A take was just made: it joins the player and the song is saved (takes autosave). */
  async function keepTake(song: WasmSong, added: TakeAdded): Promise<TakeAdded> {
    const audio = song.takeAudio(added.take);
    const take = added.project.takes.find((t) => t.id === added.take)!;
    const right = new Float32Array(audio);
    post({ kind: "addTrack", track: { left: audio, right, mix: take.mix } }, [audio.buffer, right.buffer]);
    await persist();
    return added;
  }

  /** Saves the loaded song to OPFS (a no-op where OPFS is unavailable). */
  async function persist(): Promise<void> {
    if (session.kind !== "loaded" || opfs.kind !== "available") return;
    session.id ??= crypto.randomUUID();
    await store.save(opfs.dir, session.id, session.song);
  }

  const opfs = await store.openOpfs();
  const projects: ProjectStore =
    opfs.kind === "unavailable"
      ? opfs
      : {
          kind: "available",
          list: () => store.list(opfs.dir, (project, id, savedMs) => projectSummary(project, id, savedMs) as ProjectSummary),
          async open(id) {
            return load(await store.load(opfs.dir, id), id);
          },
          async save() {
            if (session.kind !== "loaded") throw new Error("Load a song first.");
            await persist();
          },
          archive: {
            kind: "available",
            async exportSong() {
              const song = loadedSong();
              const project = song.project() as Project;
              const name = project.song.kind === "File" ? project.song.name.replace(/\.[^.]+$/, "") : "song";
              const url = URL.createObjectURL(store.archive(song));
              const a = Object.assign(document.createElement("a"), { href: url, download: `${name}.grain` });
              a.click();
              setTimeout(() => URL.revokeObjectURL(url), 60_000);
            },
            async importArchive() {
              const file = await pickFile(".grain");
              if (!file) return { kind: "cancelled" };
              void context.resume();
              const id = crypto.randomUUID();
              const song = await store.unarchive(file);
              await store.save(opfs.dir, id, song);
              return { kind: "imported", project: load(song, id) };
            },
          },
        };

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
                live: { kind: "idle" },
              };
            } catch (e) {
              mic = { kind: "closed" };
              throw e;
            }
          },
          async level() {
            return mic.kind === "recording" ? mic.mic.ring.level[0] : 0;
          },
          async live(settings: ScoreSettings) {
            if (mic.kind !== "recording") return noLive();
            const r = mic;
            if (r.live.kind === "idle") {
              const latency = await storedLatency();
              if (mic !== r || r.live.kind !== "idle") return noLive();
              // Same placement as `finish`: the cue started at capture sample 0.
              const songZero = r.countInSeconds + latency.ms / 1000 - r.fromSeconds;
              r.live = { kind: "running", session: loadedSong().liveSession(settings, context.sampleRate, songZero), fed: 0 };
            }
            const live = r.live;
            r.chunks.push(drain(r.mic.ring)); // the freshest samples, not the last timer drain
            const update = noLive();
            for (; live.fed < r.chunks.length; live.fed++) {
              const u = live.session.push(r.chunks[live.fed]) as LiveUpdate;
              update.frames.push(...u.frames);
              update.totals = u.totals;
            }
            return update;
          },
          async finish(settings: ScoreSettings) {
            if (mic.kind !== "recording") throw new Error("No take is being recorded.");
            const r = mic;
            mic = { kind: "closed" };
            window.clearInterval(r.timer);
            if (r.live.kind === "running") r.live.session.free();
            await stopPlayback();
            closeMic();
            r.chunks.push(drain(r.mic.ring));
            const capture = concat(r.chunks);
            const latency = await storedLatency();
            // The cue started at capture sample 0 (same render quantum).
            const songZero = r.countInSeconds + latency.ms / 1000 - r.fromSeconds;
            const song = loadedSong();
            const added = song.addRecordedTake(capture, context.sampleRate, songZero, latency.ms, settings) as TakeAdded;
            return keepTake(song, added);
          },
          async cancel() {
            if (mic.kind !== "recording") return;
            window.clearInterval(mic.timer);
            if (mic.live.kind === "running") mic.live.session.free();
            closeMic();
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
                closeMic();
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
    const sep = await (separation ??= loadSeparation(models));
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
    return { kind: "scored", added: await keepTake(song, added) };
  }

  return {
    chrome: { kind: "plain" },
    projects,
    async close() {
      if (microphone.kind === "available") await microphone.cancel();
      endMonitor();
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
    async setLyrics(update) {
      return loadedSong().setLyrics(update) as ProjectLyrics;
    },
    microphone,
    autotune: {
      setMonitor,
      async setTune(settings) {
        loadedSong().setTune(settings);
        post({ kind: "command", command: { kind: "Tune", settings } });
      },
      async preview(take, settings) {
        return loadedSong().tunePreview(take, settings) as TunePreview;
      },
      async apply(take, tune, settings) {
        const song = loadedSong();
        await nextFrame(); // let the UI show it's working before the render blocks
        return keepTake(song, song.applyTune(take, tune, settings) as TakeAdded);
      },
    },
    pitchEditor: {
      async open(source) {
        return loadedSong().pitchOpen(source) as PitchEditView;
      },
      async apply(edit, op) {
        return loadedSong().pitchApply(edit, op) as EditUpdate;
      },
      async set(edit) {
        return loadedSong().pitchSet(edit) as EditUpdate;
      },
      async fromTune(take, settings) {
        return loadedSong().pitchFromTune(take, settings) as EditUpdate;
      },
      async render(source, settings) {
        const song = loadedSong();
        await nextFrame(); // let the UI show it's rendering before the render blocks
        return keepTake(song, song.pitchRender(source, settings) as TakeAdded);
      },
    },
    models: {
      location: models.location,
      list: () => models.list(),
      async download(id, acceptLicense, onState) {
        const s = await models.download(id, acceptLicense, onState);
        modelsChanged();
        return s;
      },
      async delete(id) {
        const s = await models.delete(id);
        modelsChanged();
        return s;
      },
    },
    transcriber: {
      async transcribe(source) {
        const song = loadedSong();
        await nextFrame(); // let the UI show it's working before the engine blocks
        if (transcriptionMethod(source) === "DrumClassifier") return { kind: "Done", project: song.transcribeDrums(source) as Project };
        const model = await (basicPitch ??= loadBasicPitch(models));
        if (model.kind === "unavailable") {
          basicPitch = undefined;
          return { kind: "NeedsModel", model: model.model };
        }
        const [note, onset] = await model.run(song.basicPitchInput(source));
        return { kind: "Done", project: song.basicPitchFinish(source, note, onset) as Project };
      },
      async exportMidi(source) {
        downloadFile(loadedSong().midiFile(source), source.kind === "Mix" ? "Mix.mid" : `${source.stem}.mid`, "audio/midi");
        return { kind: "saved" };
      },
    },
    producer: {
      async chainPick(source, preset) {
        // The trial render takes about a second: it runs in the compute worker.
        const song = loadedSong();
        const [sampleRate, bpm] = song.chainContext();
        return compute.run<ChainPick>({ kind: "chainPick", audio: song.sourceAudio(source), sampleRate, bpm, preset });
      },
      async chainApply(source, preset, chain, settings) {
        const song = loadedSong();
        await nextFrame(); // let the UI show it's working before the render blocks
        const applied = song.chainApply(source, preset, chain, settings) as ChainApplied;
        await keepTake(song, applied.added);
        return applied;
      },
      async harmony(source, interval, settings) {
        const song = loadedSong();
        await nextFrame();
        return keepTake(song, song.harmony(source, interval, settings) as TakeAdded);
      },
      async exportMelody(source, interval) {
        downloadFile(loadedSong().melodyMidi(source, interval), melodyFileName(source, interval), "audio/midi");
        return { kind: "saved" };
      },
      denoise: {
        kind: "unavailable",
        reason: "Clean up runs DeepFilterNet in the desktop app; the web build has no runtime for it yet.",
      },
      generator: remoteGenerator(loadedSong, keepTake),
    },
    takes: {
      async rename(id, name) {
        const project = loadedSong().renameTake(id, name) as Project;
        await persist();
        return project;
      },
      async remove(id) {
        const song = loadedSong();
        await stopPlayback();
        song.deleteTake(id);
        const project = toPlayer(song);
        await persist();
        return project;
      },
    },
    async send(command) {
      await context.resume();
      // Mix settings are part of the project; a save keeps them (as on desktop).
      if (command.kind === "SetMix" && session.kind === "loaded") session.song.setMix(command.track, command.params);
      post({ kind: "command", command });
    },
    async transport() {
      return snapshot;
    },
    subscribeTransport(listener) {
      transportListeners.add(listener);
      listener(snapshot);
      return () => void transportListeners.delete(listener);
    },
  };
}

/** Offers `bytes` to the user as a download named `name`. */
function downloadFile(bytes: Uint8Array, name: string, type: string) {
  const url = URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

/** ACE-Step on the web: only through a backend configured at build time
 * (`VITE_GRAIN_GENERATION_URL`, `VITE_GRAIN_GENERATION_TOKEN`; the same HTTP contract as the
 * desktop sidecar, services/ml-sidecar/grain_sidecar/server.py). */
function remoteGenerator(loadedSong: () => WasmSong, play: (song: WasmSong, added: TakeAdded) => Promise<TakeAdded>): Generator {
  const url = import.meta.env.VITE_GRAIN_GENERATION_URL as string | undefined;
  const token = import.meta.env.VITE_GRAIN_GENERATION_TOKEN as string | undefined;
  if (!url || !token) {
    return {
      kind: "unavailable",
      reason: "AI generation runs ACE-Step locally in the desktop app (about 8 GB). The web build has no backend configured.",
    };
  }
  const ready: GeneratorState = { kind: "Ready" };
  return {
    kind: "available",
    where: "remote",
    expectations: `Runs on ${new URL(url).host}; the song's vocal or mix is uploaded there for each generation.`,
    state: async () => ready,
    install: async () => ready,
    uninstall: async () => ready,
    async generate(task, prompt, seed, settings, onState) {
      const song = loadedSong();
      const request = song.generationRequest(task, prompt, seed) as GenerationRequest;
      const input = song.generationInput(task);
      const project = song.project() as Project;
      const query = new URLSearchParams(generationQuery(request, project.frames / project.sample_rate) as [string, string][]);
      onState({ kind: "Running" });
      try {
        const r = await fetch(`${url.replace(/\/$/, "")}/generate?${query}`, {
          method: "POST",
          headers: { Authorization: `Bearer ${token}`, "Content-Type": "audio/wav" },
          body: new Uint8Array(input),
        });
        if (!r.ok) throw new Error(`generation failed (${r.status}): ${await r.text()}`);
        return play(song, song.addGenerated(request, new Uint8Array(await r.arrayBuffer()), settings) as TakeAdded);
      } finally {
        onState(ready);
      }
    },
  };
}

/** HTDemucs through onnxruntime-web (WebGPU, falling back to WASM) if the model is installed. */
async function loadSeparation(models: WebModels): Promise<Separation> {
  const [specFile, onnx] = await Promise.all([models.file(`${MODEL}.json`), models.file(`${MODEL}.onnx`)]);
  if (!specFile || !onnx) return { kind: "unavailable" };
  const spec = await specFile.text();
  const ort = await import("onnxruntime-web/webgpu");
  const session = await ort.InferenceSession.create(new Uint8Array(await onnx.arrayBuffer()), {
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

type BasicPitch =
  | { kind: "available"; run(windows: Float32Array): Promise<[Float32Array, Float32Array]> }
  | { kind: "unavailable"; model: string };

/** Windows per basic-pitch call (bounds memory on long songs). */
const BASIC_PITCH_BATCH = 8;

/** basic-pitch through onnxruntime-web (WASM: the model is small) if the model is installed. */
async function loadBasicPitch(models: WebModels): Promise<BasicPitch> {
  const spec = basicPitchSpec() as { model: string; file: string; input: string; note: string; onset: string; window: number };
  const onnx = await models.file(spec.file);
  if (!onnx) return { kind: "unavailable", model: spec.model };
  const ort = await import("onnxruntime-web/webgpu");
  const session = await ort.InferenceSession.create(new Uint8Array(await onnx.arrayBuffer()), { executionProviders: ["wasm"] });
  return {
    kind: "available",
    async run(windows) {
      const note: Float32Array[] = [];
      const onset: Float32Array[] = [];
      const step = BASIC_PITCH_BATCH * spec.window;
      for (let a = 0; a < windows.length; a += step) {
        const chunk = windows.subarray(a, Math.min(windows.length, a + step));
        const out = await session.run({ [spec.input]: new ort.Tensor("float32", chunk, [chunk.length / spec.window, spec.window, 1]) });
        note.push((await out[spec.note].getData()) as Float32Array);
        onset.push((await out[spec.onset].getData()) as Float32Array);
      }
      return [concat(note), concat(onset)];
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
