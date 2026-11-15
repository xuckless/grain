// Desktop engine host: the Rust backend (engine-native) behind Tauri commands.

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open, save } from "@tauri-apps/plugin-dialog";
import type {
  EngineHost,
  ImportOutcome,
  ImportStage,
  LiveUpdate,
  Latency,
  MonitorStatus,
  Project,
  ProjectLyrics,
  ProjectSummary,
  ScoreReport,
  SeparationOffer,
  StageListener,
  TakeAdded,
  TransportSnapshot,
  TunePreview,
  EditUpdate,
  PitchEditView,
  ModelEntry,
  ModelState,
  TrackSource,
  TranscribeOutcome,
  WindowChrome,
  ExtractorOrigin,
  ExtractorUpdate,
  ChainOutcome,
  ChainPick,
  ExportFormat,
  ExportProgress,
  FormatOffer,
  GeneratorState,
} from "@workstation/engine-api";
import { melodyFileName } from "@workstation/engine-api";

/** What ACE-Step costs here (AGENTS.md §8); estimates from its documentation, not measured. */
const GENERATION_EXPECTATIONS =
  "Installing downloads about 2 GB of Python packages and 6.3 GB of ACE-Step weights (keep 10 GB free). " +
  "Each generation starts the model (about 30 s) and frees it afterwards. On Apple silicon (MPS) expect a few minutes " +
  "and up to about 10 GB of memory for a 3-minute song; on CPU only, much longer.";

/** Runs `run` while relaying "generator-state" events to `onState`. */
async function withGeneratorStates<T>(onState: (s: GeneratorState) => void, run: () => Promise<T>): Promise<T> {
  const unlisten = await listen<GeneratorState>("generator-state", (e) => onState(e.payload));
  try {
    return await run();
  } finally {
    unlisten();
  }
}

/** macOS draws its window buttons over the page (tauri.conf.json: titleBarStyle "Overlay");
 * the top bar leaves this much room for them. Other platforms keep native decorations. */
const MAC_BUTTONS_INSET = 88;
const chrome: WindowChrome = navigator.userAgent.includes("Mac OS")
  ? {
      kind: "overlay",
      insetLeft: MAC_BUTTONS_INSET,
      startDrag: () => void getCurrentWindow().startDragging(),
      toggleMaximize: () => void getCurrentWindow().toggleMaximize(),
    }
  : { kind: "plain" };

const AUDIO = [{ name: "Audio", extensions: ["mp3", "m4a", "wav", "flac", "ogg", "aac"] }];

async function pickAudio(): Promise<string | undefined> {
  const path = await open({ multiple: false, directory: false, filters: AUDIO });
  return typeof path === "string" ? path : undefined;
}

/** Runs an import command while forwarding the backend's `import-stage` events. */
async function withStages(onStage: StageListener, run: () => Promise<Project>): Promise<ImportOutcome> {
  const unlisten = await listen<ImportStage>("import-stage", (e) => onStage(e.payload));
  try {
    return { kind: "imported", project: await run() };
  } finally {
    unlisten();
  }
}

export async function createDesktopHost(): Promise<EngineHost> {
  const urlImport = await invoke<
    { kind: "available"; origin: ExtractorOrigin } | { kind: "missing"; reason: string } | { kind: "unavailable" }
  >("url_import");
  const separation = await invoke<SeparationOffer>("separation_offer");
  /** The last player command sent; the next waits for it. */
  let sendQueue: Promise<unknown> = Promise.resolve();
  return {
    chrome,
    projects: {
      kind: "available",
      list: () => invoke<ProjectSummary[]>("list_projects"),
      open: (id) => invoke<Project>("open_project", { id }),
      save: () => invoke("save_project"),
      archive: {
        kind: "available",
        async exportSong() {
          const path = await save({ defaultPath: "Project.grain", filters: [{ name: "Grain project", extensions: ["grain"] }] });
          if (path) await invoke("export_archive", { path });
        },
        async importArchive(): Promise<ImportOutcome> {
          const path = await open({ multiple: false, filters: [{ name: "Grain project", extensions: ["grain"] }] });
          if (typeof path !== "string") return { kind: "cancelled" };
          return { kind: "imported", project: await invoke<Project>("import_archive", { path }) };
        },
      },
    },
    close: () => invoke("close_session"),
    urlImport:
      urlImport.kind === "available"
        ? {
            kind: "available",
            extractor: {
              origin: urlImport.origin,
              version: () => invoke<string>("extractor_version"),
              update: () => invoke<ExtractorUpdate>("update_extractor"),
            },
            importUrl: (url, quality, onStage) => withStages(onStage, () => invoke<Project>("import_url", { url, quality })),
          }
        : urlImport,
    separation,
    async importFile(quality, onStage) {
      const path = await pickAudio();
      if (!path) return { kind: "cancelled" };
      return withStages(onStage, () => invoke<Project>("import_file", { path, quality }));
    },
    async scoreTake(settings) {
      const path = await pickAudio();
      if (!path) return { kind: "cancelled" };
      return { kind: "scored", added: await invoke<TakeAdded>("score_take", { path, settings }) };
    },
    scoreRecorded: (take, settings) => invoke<ScoreReport>("score_recorded", { take, settings }),
    setLyrics: (update) => invoke<ProjectLyrics>("set_lyrics", { update }),
    microphone: {
      kind: "available",
      start: () => invoke("start_recording"),
      level: () => invoke<number>("capture_level"),
      live: (settings) => invoke<LiveUpdate>("live_pitch", { settings }),
      finish: (settings) => invoke<TakeAdded>("finish_recording", { settings }),
      cancel: () => invoke("cancel_recording"),
      latency: () => invoke<Latency>("latency"),
      setLatency: (ms) => invoke<Latency>("set_latency", { ms }),
      measureLatency: () => invoke<Latency>("measure_latency"),
    },
    autotune: {
      setMonitor: (mode) => invoke<MonitorStatus>("set_monitor", { mode }),
      setTune: (settings) => invoke("set_tune", { settings }),
      preview: (take, settings) => invoke<TunePreview>("tune_preview", { take, settings }),
    },
    pitchEditor: {
      open: (source) => invoke<PitchEditView>("pitch_open", { source }),
      apply: (edit, op) => invoke<EditUpdate>("pitch_apply", { edit, op }),
      set: (edit) => invoke<EditUpdate>("pitch_set", { edit }),
      fromTune: (take, settings) => invoke<EditUpdate>("pitch_from_tune", { take, settings }),
    },
    models: {
      location: "the app's data folder",
      list: () => invoke<ModelEntry[]>("models_list"),
      async download(id, acceptLicense, onState) {
        const unlisten = await listen<{ id: string; state: ModelState }>("model-state", (e) => {
          if (e.payload.id === id) onState(e.payload.state);
        });
        try {
          return await invoke<ModelState>("model_download", { id, acceptLicense });
        } finally {
          unlisten();
        }
      },
      delete: (id) => invoke<ModelState>("model_delete", { id }),
    },
    transcriber: {
      transcribe: (source) => invoke<TranscribeOutcome>("transcribe", { source }),
      async exportMidi(source: TrackSource) {
        const name = source.kind === "Mix" ? "Mix.mid" : `${source.stem}.mid`;
        const path = await save({ defaultPath: name, filters: [{ name: "MIDI", extensions: ["mid"] }] });
        if (!path) return { kind: "cancelled" };
        await invoke("export_midi", { source, path });
        return { kind: "saved" };
      },
    },
    producer: {
      chainPick: (from, preset) => invoke<ChainPick>("chain_pick", { from, preset }),
      harmony: (from, interval, settings) => invoke<TakeAdded>("harmony", { from, interval, settings }),
      async exportMelody(from, interval) {
        const path = await save({ defaultPath: melodyFileName(from, interval), filters: [{ name: "MIDI", extensions: ["mid"] }] });
        if (!path) return { kind: "cancelled" };
        await invoke("export_melody", { from, interval, path });
        return { kind: "saved" };
      },
      denoise: { kind: "available", model: "deepfilternet3" },
      generator: {
        kind: "available",
        where: "local",
        expectations: GENERATION_EXPECTATIONS,
        state: () => invoke<GeneratorState>("generator_state"),
        install: (onState) => withGeneratorStates(onState, () => invoke<GeneratorState>("generator_install")),
        uninstall: () => invoke<GeneratorState>("generator_uninstall"),
        generate: (task, prompt, seed, settings, onState) =>
          withGeneratorStates(onState, () => invoke<TakeAdded>("generate", { task, prompt, seed, settings })),
      },
    },
    takes: {
      rename: (id, name) => invoke<Project>("rename_take", { id, name }),
      remove: (id) => invoke<Project>("delete_take", { id }),
      setChain: (id, chain) => invoke<ChainOutcome>("set_take_chain", { id, chain }),
      setMaster: (id) => invoke<Project>("set_master", { id }),
    },
    exporter: {
      async formats(): Promise<FormatOffer[]> {
        const list = await invoke<[ExportFormat, string | null][]>("export_formats");
        return list.map(([format, reason]) => ({ format, availability: reason ? { kind: "unavailable", reason } : { kind: "ready" } }));
      },
      async run(request, name, onProgress) {
        const ext = { Wav24: "wav", Mp3: "mp3", M4a: "m4a" }[request.format];
        const path = await save({ defaultPath: `${name}.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
        if (!path) return { kind: "cancelled" };
        const progress = new Channel<ExportProgress>();
        progress.onmessage = onProgress;
        await invoke("export_mix", { request, path, progress });
        return { kind: "saved", path };
      },
    },
    send(command) {
      // Commands run off the main thread (AGENTS.md §10.1); chaining keeps their order.
      const sent = sendQueue.then(() => invoke<void>("player", { command }));
      sendQueue = sent.catch(() => {});
      return sent;
    },
    transport: () => invoke<TransportSnapshot>("transport"),
    subscribeTransport(listener) {
      // The backend pushes `transport` events (see `push_transport`); read once to start.
      let alive = true;
      const unlisten = listen<TransportSnapshot>("transport", (e) => alive && listener(e.payload));
      void invoke<TransportSnapshot>("transport").then((s) => alive && listener(s));
      return () => {
        alive = false;
        void unlisten.then((f) => f());
      };
    },
  };
}
