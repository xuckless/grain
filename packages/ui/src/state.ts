// Every UI state is declared here (AGENTS.md ground rules 1–2).

import type { ImportStage, Project, ScoreReport } from "@workstation/engine-api";

/** Import progress: `Starting` until the host reports its first engine stage. */
export type ImportProgress = { kind: "Starting" } | ImportStage;

export type ScoreState =
  | { kind: "none" }
  | { kind: "scoring" }
  | { kind: "scored"; report: ScoreReport }
  | { kind: "failed"; message: string };

export type AppState =
  | { kind: "empty" }
  | { kind: "importing"; progress: ImportProgress }
  | { kind: "ready"; project: Project; score: ScoreState }
  | { kind: "failed"; message: string };

export function describeProgress(p: ImportProgress): string {
  switch (p.kind) {
    case "Starting":
      return "Opening…";
    case "Decoding":
      return "Decoding audio…";
    case "Separating":
      return `Separating stems: segment ${p.segment + 1} of ${p.of}`;
    case "Analyzing":
      return "Analyzing tempo, key and vocal melody…";
    case "Caching":
      return "Saving stems…";
  }
}

export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** Seconds as m:ss.t */
export function formatTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds - 60 * m;
  return `${m}:${s.toFixed(1).padStart(4, "0")}`;
}
