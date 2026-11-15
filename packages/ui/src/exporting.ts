// Export (AGENTS.md §8.8), UI side: which tracks the dialog offers and ticks by default, and the
// dialog's state machine. Pure, unit-tested under Node.

import type { ExportFormat, Project, TrackMode } from "@workstation/engine-api";
import { masterId } from "./chain.ts";
import { sungTakes } from "./listen.ts";
import { vocalTrack } from "./takes.ts";

/** One checklist row: a player track index (song tracks, then takes). */
export type ExportTrack = { index: number; label: string; checked: boolean };

const audible = (mode: TrackMode, anySolo: boolean) => mode === "Soloed" || (mode === "Normal" && !anySolo);

/** The tracks offered, ticked as the mix is heard now (mute and solo), with the ★ master take
 * in. The song's separated vocal (the guide vocal) is never offered; other sung takes are
 * offered unticked. */
export function exportTracks(project: Project): ExportTrack[] {
  const v = vocalTrack(project);
  const guide = v >= 0 && project.tracks[v].source.kind === "Stem" ? v : -1;
  const sung = new Set(sungTakes(project).map((t) => t.id));
  const master = masterId(project);
  const anySolo = [...project.tracks.map((t) => t.mix.mode), ...project.takes.filter((t) => !sung.has(t.id)).map((t) => t.mix.mode)].includes("Soloed");
  const tracks = project.tracks
    .map((t, index) => ({ index, label: t.name, checked: audible(t.mix.mode, anySolo) }))
    .filter((t) => t.index !== guide);
  const takes = project.takes.map((t, i) => {
    const index = project.tracks.length + i;
    if (sung.has(t.id)) return { index, label: t.id === master ? `★ ${t.name}` : t.name, checked: t.id === master };
    return { index, label: t.name, checked: audible(t.mix.mode, anySolo) };
  });
  return [...tracks, ...takes];
}

export const FORMAT_LABEL: Record<ExportFormat, string> = { Wav24: "WAV (24-bit)", Mp3: "MP3 (320 kbps)", M4a: "M4A (AAC 256 kbps)" };

export type ExportState =
  | { kind: "idle" }
  | { kind: "rendering"; fraction: number }
  | { kind: "encoding" }
  | { kind: "done"; path: string }
  | { kind: "failed"; message: string };

export type ExportEvent =
  | { kind: "start" }
  | { kind: "progress"; fraction: number }
  | { kind: "encoding" }
  | { kind: "saved"; path: string }
  | { kind: "cancelled" }
  | { kind: "fail"; message: string };

/** The dialog's next state, or `null` when the event doesn't apply (one export at a time). */
export function exportNext(s: ExportState, e: ExportEvent): ExportState | null {
  const running = s.kind === "rendering" || s.kind === "encoding";
  switch (e.kind) {
    case "start":
      return running ? null : { kind: "rendering", fraction: 0 };
    case "progress":
      return s.kind === "rendering" ? { kind: "rendering", fraction: e.fraction } : null;
    case "encoding":
      return running ? { kind: "encoding" } : null;
    case "saved":
      return running ? { kind: "done", path: e.path } : null;
    case "cancelled":
      return running ? { kind: "idle" } : null;
    case "fail":
      return running ? { kind: "failed", message: e.message } : null;
  }
}
