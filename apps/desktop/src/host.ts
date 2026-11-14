// Desktop engine host: the Rust backend (engine-native) behind Tauri commands.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  EngineHost,
  ImportOutcome,
  ImportStage,
  Latency,
  Project,
  ProjectSummary,
  ScoreReport,
  SeparationOffer,
  StageListener,
  TakeAdded,
  TransportSnapshot,
  WindowChrome,
} from "@workstation/engine-api";

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
  const urlImport = await invoke<{ kind: "available" } | { kind: "unavailable" }>("url_import");
  const separation = await invoke<SeparationOffer>("separation_offer");
  return {
    chrome,
    projects: {
      kind: "available",
      list: () => invoke<ProjectSummary[]>("list_projects"),
      open: (id) => invoke<Project>("open_project", { id }),
      save: () => invoke("save_project"),
      // The stem cache is the desktop's project store; archives are a web feature for now.
      archive: { kind: "unavailable" },
    },
    close: () => invoke("close_session"),
    urlImport:
      urlImport.kind === "available"
        ? {
            kind: "available",
            importUrl: (url, quality, onStage) => withStages(onStage, () => invoke<Project>("import_url", { url, quality })),
          }
        : { kind: "unavailable" },
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
    microphone: {
      kind: "available",
      start: () => invoke("start_recording"),
      level: () => invoke<number>("capture_level"),
      finish: (settings) => invoke<TakeAdded>("finish_recording", { settings }),
      cancel: () => invoke("cancel_recording"),
      latency: () => invoke<Latency>("latency"),
      setLatency: (ms) => invoke<Latency>("set_latency", { ms }),
      measureLatency: () => invoke<Latency>("measure_latency"),
    },
    send: (command) => invoke("player", { command }),
    transport: () => invoke<TransportSnapshot>("transport"),
  };
}
