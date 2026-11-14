// Desktop engine host: the Rust backend (engine-native) behind Tauri commands.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  EngineHost,
  ImportOutcome,
  ImportStage,
  Project,
  ProjectSummary,
  ScoreReport,
  StageListener,
  TransportSnapshot,
  WindowChrome,
} from "@workstation/engine-api";

/** macOS draws its window buttons over the page (tauri.conf.json: titleBarStyle "Overlay");
 * the top bar leaves this much room for them. Other platforms keep native decorations. */
const MAC_BUTTONS_INSET = 88;
const chrome: WindowChrome = navigator.userAgent.includes("Mac OS")
  ? { kind: "overlay", insetLeft: MAC_BUTTONS_INSET }
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
  return {
    chrome,
    projects: {
      kind: "available",
      list: () => invoke<ProjectSummary[]>("list_projects"),
      open: (id) => invoke<Project>("open_project", { id }),
      save: () => invoke("save_project"),
    },
    close: () => invoke("close_session"),
    urlImport:
      urlImport.kind === "available"
        ? { kind: "available", importUrl: (url, onStage) => withStages(onStage, () => invoke<Project>("import_url", { url })) }
        : { kind: "unavailable" },
    async importFile(onStage) {
      const path = await pickAudio();
      if (!path) return { kind: "cancelled" };
      return withStages(onStage, () => invoke<Project>("import_file", { path }));
    },
    async scoreTake(settings) {
      const path = await pickAudio();
      if (!path) return { kind: "cancelled" };
      return { kind: "scored", report: await invoke<ScoreReport>("score_take", { path, settings }) };
    },
    send: (command) => invoke("player", { command }),
    transport: () => invoke<TransportSnapshot>("transport"),
  };
}
