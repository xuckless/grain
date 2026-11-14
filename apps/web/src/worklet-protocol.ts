// Messages between the main thread and the player AudioWorklet.

import type { MixParams, PlayerCommand, Transport } from "@workstation/engine-api";

export type TrackData = { left: Float32Array; right: Float32Array; mix: MixParams };

export type ToWorklet =
  | { kind: "init"; module: WebAssembly.Module }
  | { kind: "load"; tracks: TrackData[] }
  | { kind: "unload" }
  | { kind: "command"; command: PlayerCommand }
  /** Adds a take as a player track. */
  | { kind: "addTrack"; track: TrackData }
  /** Starts writing the microphone (the node's input) into `ring` and applies `command` (a
   * count-in or the calibration chirp) in the same render quantum, so the cue starts at
   * capture sample 0. */
  | { kind: "capture"; ring: SharedArrayBuffer; command: PlayerCommand }
  | { kind: "endCapture" };

export type FromWorklet =
  | { kind: "ready" }
  | { kind: "transport"; transport: Transport }
  | { kind: "rejected"; message: string }
  /** The cue started: the transport right after it (a count-in holds the song position). */
  | { kind: "cueStarted"; transport: Transport };
