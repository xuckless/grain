// Every app-level UI state is declared here (AGENTS.md ground rules 1–2). Component-local
// states (menus, sliders, toasts) are declared as unions in their components.

import type { ImportStage, Project, ScoreReport } from "@workstation/engine-api";

/** Import progress: `Starting` until the host reports its first engine stage. */
export type ImportProgress = { kind: "Starting" } | ImportStage;

export type ScoreState =
  | { kind: "none" }
  | { kind: "scoring" }
  | { kind: "scored"; report: ScoreReport }
  | { kind: "failed"; message: string };

/** Whether the loaded song's mix differs from its saved project. */
export type SaveState = { kind: "clean" } | { kind: "dirty" } | { kind: "saving" };

export type ViewId = "Mix" | "Score" | "Karaoke" | "AutoTune" | "Pitch" | "Produce";

export type Availability = { kind: "ready" } | { kind: "unavailable"; reason: string };

export type ViewDef = { id: ViewId; label: string; availability: Availability };

/** Session views in tab order (AGENTS.md roadmap). */
export const VIEWS: readonly ViewDef[] = [
  { id: "Mix", label: "Mix", availability: { kind: "ready" } },
  { id: "Score", label: "Score", availability: { kind: "ready" } },
  { id: "Karaoke", label: "Karaoke", availability: { kind: "unavailable", reason: "Live lyrics and a pitch lane, coming in M2." } },
  { id: "AutoTune", label: "Auto-tune", availability: { kind: "unavailable", reason: "Pitch correction to the song's key, coming in M3." } },
  { id: "Pitch", label: "Pitch", availability: { kind: "unavailable", reason: "Note-by-note pitch editing, coming in M4." } },
  { id: "Produce", label: "Produce", availability: { kind: "unavailable", reason: "AI vocal enhancement and backing tracks, coming in M5." } },
];

export type Session = { project: Project; score: ScoreState; save: SaveState; view: ViewId };

export function newSession(project: Project): Session {
  return { project, score: { kind: "none" }, save: { kind: "clean" }, view: "Mix" };
}

/** What the Home screen is doing. */
export type HomeJob =
  | { kind: "idle" }
  | { kind: "importing"; progress: ImportProgress }
  | { kind: "opening" }
  | { kind: "failed"; message: string };

/** A session left open behind the Home screen (after Back). */
export type Resume = { kind: "none" } | { kind: "available"; session: Session };

export type AppState = { kind: "home"; job: HomeJob; resume: Resume } | { kind: "session"; session: Session };

export function describeProgress(p: ImportProgress): string {
  switch (p.kind) {
    case "Starting":
      return "Opening";
    case "Decoding":
      return "Decoding audio";
    case "Separating":
      return `Separating stems, segment ${p.segment + 1} of ${p.of}`;
    case "Analyzing":
      return "Finding tempo, key and the vocal melody";
    case "Caching":
      return "Saving stems";
  }
}

/** Overall import progress 0–1 (separation dominates the time). */
export function progressFraction(p: ImportProgress): number {
  switch (p.kind) {
    case "Starting":
      return 0;
    case "Decoding":
      return 0.03;
    case "Separating":
      return 0.05 + 0.85 * (p.segment / Math.max(1, p.of));
    case "Analyzing":
      return 0.92;
    case "Caching":
      return 0.98;
  }
}

export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

const pad2 = (n: number) => String(n).padStart(2, "0");

/** Readout time, `01:24.3`. */
export function formatTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds - 60 * m;
  return `${pad2(m)}:${s.toFixed(1).padStart(4, "0")}`;
}

/** Song length, `3:45`. */
export function formatDuration(seconds: number): string {
  const m = Math.floor(seconds / 60);
  return `${m}:${pad2(Math.floor(seconds - 60 * m))}`;
}

export function songTitle(project: Pick<Project, "song">): string {
  return project.song.kind === "File" ? project.song.name.replace(/\.[^.]+$/, "") : project.song.title;
}

/** Linear gain as a dB readout, `−6.0 dB`. */
export function formatDb(gain: number): string {
  if (gain <= 0.0001) return "−∞ dB";
  const db = 20 * Math.log10(gain);
  return `${db < 0 ? "−" : db > 0 ? "+" : ""}${Math.abs(db).toFixed(1)} dB`;
}

export function formatPan(pan: number): string {
  const p = Math.round(pan * 100);
  return p === 0 ? "C" : p < 0 ? `L${-p}` : `R${p}`;
}
