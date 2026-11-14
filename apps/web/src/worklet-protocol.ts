// Messages between the main thread and the player AudioWorklet.

import type { MixParams, PlayerCommand, Transport } from "@workstation/engine-api";

export type TrackData = { left: Float32Array; right: Float32Array; mix: MixParams };

export type ToWorklet =
  | { kind: "init"; module: WebAssembly.Module }
  | { kind: "load"; tracks: TrackData[] }
  | { kind: "unload" }
  | { kind: "command"; command: PlayerCommand };

export type FromWorklet =
  | { kind: "ready" }
  | { kind: "transport"; transport: Transport }
  | { kind: "rejected"; message: string };
