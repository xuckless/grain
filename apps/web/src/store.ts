// Web project store (AGENTS.md §12). One `.grain` file per project is both the OPFS record
// (`projects/<id>.grain`) and the exported archive:
//   8 bytes  magic "GRAINv1\n"
//   u32 LE   header length H
//   H bytes  UTF-8 JSON { "project": Project }
//   then 16-bit little-endian PCM channels, each `project.frames` samples, in project order:
//   every track's left then right channel, then every take's timeline audio (mono).

import type { Project, ProjectSummary } from "@workstation/engine-api";
import { SongRestorer, type WasmSong } from "engine-wasm";

const MAGIC = new TextEncoder().encode("GRAINv1\n");
const DIR = "projects";
const EXT = ".grain";

export type Opfs = { kind: "available"; dir: FileSystemDirectoryHandle } | { kind: "unavailable"; reason: string };

export async function openOpfs(): Promise<Opfs> {
  const reason = "This browser can't store projects (Origin Private File System unavailable).";
  try {
    if (!navigator.storage?.getDirectory || !("createWritable" in FileSystemFileHandle.prototype)) return { kind: "unavailable", reason };
    const root = await navigator.storage.getDirectory();
    return { kind: "available", dir: await root.getDirectoryHandle(DIR, { create: true }) };
  } catch {
    return { kind: "unavailable", reason };
  }
}

/** Channels of a song in file order. */
function* channels(song: WasmSong, project: Project): Generator<Float32Array> {
  for (let i = 0; i < project.tracks.length; i++) {
    const [left, right] = song.trackAudio(i) as [Float32Array, Float32Array];
    yield left;
    yield right;
  }
  for (const t of project.takes) yield song.takeAudio(t.id);
}

function pcm16(x: Float32Array): Int16Array<ArrayBuffer> {
  const out = new Int16Array(x.length);
  for (let i = 0; i < x.length; i++) out[i] = Math.max(-32768, Math.min(32767, Math.round(x[i] * 32767)));
  return out;
}

function float(x: Int16Array): Float32Array {
  const out = new Float32Array(x.length);
  for (let i = 0; i < x.length; i++) out[i] = x[i] / 32767;
  return out;
}

function header(project: Project): Uint8Array<ArrayBuffer> {
  const json = new TextEncoder().encode(JSON.stringify({ project }));
  const out = new Uint8Array(MAGIC.length + 4 + json.length);
  out.set(MAGIC);
  new DataView(out.buffer).setUint32(MAGIC.length, json.length, true);
  out.set(json, MAGIC.length + 4);
  return out;
}

/** A song as one `.grain` file (parts are converted one channel at a time). */
export function archive(song: WasmSong): Blob {
  const project = song.project() as Project;
  return new Blob([header(project), ...Array.from(channels(song, project), pcm16)], { type: "application/octet-stream" });
}

async function readHeader(file: Blob): Promise<{ project: Project; audioStart: number }> {
  const head = new Uint8Array(await file.slice(0, MAGIC.length + 4).arrayBuffer());
  if (head.length < MAGIC.length + 4 || MAGIC.some((b, i) => head[i] !== b)) throw new Error("This isn't a Grain project file.");
  const length = new DataView(head.buffer).getUint32(MAGIC.length, true);
  const start = MAGIC.length + 4;
  const json = await file.slice(start, start + length).text();
  return { project: (JSON.parse(json) as { project: Project }).project, audioStart: start + length };
}

/** Reopens a `.grain` file, reading one channel at a time. */
export async function unarchive(file: Blob): Promise<WasmSong> {
  const { project, audioStart } = await readHeader(file);
  const bytes = project.frames * 2;
  const expected = audioStart + bytes * (2 * project.tracks.length + project.takes.length);
  if (file.size !== expected) throw new Error("The project file is incomplete.");
  let at = audioStart;
  const next = async () => {
    const x = float(new Int16Array(await file.slice(at, at + bytes).arrayBuffer()));
    at += bytes;
    return x;
  };
  const restorer = new SongRestorer(project);
  for (let i = 0; i < project.tracks.length; i++) restorer.pushTrack(await next(), await next());
  for (let i = 0; i < project.takes.length; i++) restorer.pushTake(await next());
  return restorer.finish();
}

export async function save(dir: FileSystemDirectoryHandle, id: string, song: WasmSong): Promise<void> {
  const handle = await dir.getFileHandle(id + EXT, { create: true });
  const writable = await handle.createWritable();
  try {
    await writable.write(archive(song));
    await writable.close();
  } catch (e) {
    await writable.abort();
    throw e;
  }
}

export async function load(dir: FileSystemDirectoryHandle, id: string): Promise<WasmSong> {
  return unarchive(await (await dir.getFileHandle(id + EXT)).getFile());
}

/** Saved projects, newest first. Unreadable files are skipped. */
export async function list(dir: FileSystemDirectoryHandle, summary: (project: Project, id: string, savedMs: number) => ProjectSummary): Promise<ProjectSummary[]> {
  const out: ProjectSummary[] = [];
  for await (const [name, handle] of dir as unknown as AsyncIterable<[string, FileSystemHandle]>) {
    if (handle.kind !== "file" || !name.endsWith(EXT)) continue;
    try {
      const file = await (handle as FileSystemFileHandle).getFile();
      out.push(summary((await readHeader(file)).project, name.slice(0, -EXT.length), file.lastModified));
    } catch {
      // Not a readable project file.
    }
  }
  return out.sort((a, b) => b.saved_ms - a.saved_ms);
}
