// Web model store for the model manager (AGENTS.md §10): manifest models downloaded into OPFS
// (`models/`), streamed through the engine's SHA-256 and marked usable (`<id>.ok`) only once every
// file verified. The registry and state machine are the engine's (engine-ml::models via wasm).
// Development servers also serve the repository's weights at /models; those count as ready.

import type { ModelEntry, ModelEvent, ModelInfo, ModelState } from "@workstation/engine-api";
import { ModelChecksum, modelManifest, modelNext, modelUrlAllowed } from "engine-wasm";
import { openOpfs, type Opfs } from "./store";

const DIR = "models";
const marker = (id: string) => `${id}.ok`;

export class WebModels {
  readonly location = "this browser's private storage for the app";
  /** Models downloading or verifying, or whose last download failed. */
  private active = new Map<string, ModelState>();
  private opfs: Promise<Opfs> = openOpfs(DIR, "models");

  private manifest(): ModelInfo[] {
    return modelManifest() as ModelInfo[];
  }

  private info(id: string): ModelInfo {
    const m = this.manifest().find((m) => m.id === id);
    if (!m) throw new Error(`unknown model ${id}`);
    return m;
  }

  private async dir(): Promise<FileSystemDirectoryHandle> {
    const o = await this.opfs;
    if (o.kind === "unavailable") throw new Error(o.reason);
    return o.dir;
  }

  private async stored(id: string): Promise<boolean> {
    try {
      await (await this.dir()).getFileHandle(marker(id));
      return true;
    } catch {
      return false;
    }
  }

  private async state(info: ModelInfo): Promise<ModelState> {
    const a = this.active.get(info.id);
    if (a) return a;
    if ((await this.stored(info.id)) || (await served(info))) return { kind: "Ready" };
    return { kind: "NotDownloaded" };
  }

  private async advance(info: ModelInfo, event: ModelEvent): Promise<ModelState> {
    const current = await this.state(info);
    const next = modelNext(current, event) as ModelState | null;
    if (!next) return current;
    if (next.kind === "Ready" || next.kind === "NotDownloaded") this.active.delete(info.id);
    else this.active.set(info.id, next);
    return next;
  }

  async list(): Promise<ModelEntry[]> {
    return Promise.all(this.manifest().map(async (info) => ({ info, state: await this.state(info) })));
  }

  async download(id: string, acceptLicense: boolean, onState: (s: ModelState) => void): Promise<ModelState> {
    const info = this.info(id);
    if (info.obtain.kind !== "Download") throw new Error(`${id} can't be downloaded in the app`);
    if (info.distribution === "opt-in" && !acceptLicense) throw new Error(`${id} is under ${info.license}: accept its licence first`);
    const step = async (e: ModelEvent) => {
      const s = await this.advance(info, e);
      onState(s);
      return s;
    };
    const total = info.files.reduce((n, f) => n + (f.size ?? 0), 0);
    await step({ kind: "Start", total: total });
    const dir = await this.dir();
    try {
      let received = 0;
      const sums: ModelChecksum[] = [];
      for (const f of info.files) {
        if (!f.url || !modelUrlAllowed(f.url)) throw new Error(`${f.name} isn't on a model host`);
        const response = await fetch(f.url);
        if (!response.ok || !response.body) throw new Error(`${f.name}: HTTP ${response.status}`);
        const out = await (await dir.getFileHandle(f.name, { create: true })).createWritable();
        const sum = new ModelChecksum();
        const reader = response.body.getReader();
        let reported = 0;
        for (;;) {
          const { done, value } = await reader.read();
          if (done) break;
          sum.update(value);
          await out.write(value);
          received += value.length;
          if (received - reported > 1 << 20) {
            reported = received;
            await step({ kind: "Progress", received: received });
          }
        }
        await out.close();
        sums.push(sum);
      }
      await step({ kind: "Downloaded" });
      const bad = info.files.find((f, i) => !sums[i].verify(f));
      sums.forEach((s) => s.free());
      if (bad) throw new Error(`${bad.name} failed its checksum (the download was corrupted or the file changed upstream)`);
      await (await (await dir.getFileHandle(marker(id), { create: true })).createWritable()).close();
      return await step({ kind: "Verified" });
    } catch (e) {
      await this.removeFiles(info);
      return await step({ kind: "Fail", message: e instanceof Error ? e.message : String(e) });
    }
  }

  private async removeFiles(info: ModelInfo): Promise<void> {
    const dir = await this.dir();
    for (const name of [marker(info.id), ...info.files.map((f) => f.name)]) await dir.removeEntry(name).catch(() => {});
  }

  async delete(id: string): Promise<ModelState> {
    const info = this.info(id);
    await this.removeFiles(info);
    return this.advance(info, { kind: "Deleted" });
  }

  /** Model file `name`'s bytes, from OPFS (verified) or the development server; null if absent. */
  async file(name: string): Promise<Blob | null> {
    const info = this.manifest().find((m) => m.files.some((f) => f.name === name));
    if (info && (await this.stored(info.id))) return (await (await this.dir()).getFileHandle(name)).getFile();
    const r = await fetch(`/models/${name}`);
    return isModelResponse(r) ? r.blob() : null;
  }
}

/** Whether the development server serves every file of `info` at /models. */
async function served(info: ModelInfo): Promise<boolean> {
  for (const f of info.files) {
    const r = await fetch(`/models/${f.name}`, { method: "HEAD" }).catch(() => null);
    if (!r || !isModelResponse(r)) return false;
  }
  return true;
}

/** A real file, not a single-page-app fallback to index.html. */
function isModelResponse(r: Response): boolean {
  return r.ok && !(r.headers.get("content-type") ?? "").includes("text/html");
}
