// The song's playhead, per frame, without per-frame engine traffic (AGENTS.md §10.1).
//
// Hosts push the transport (~40 Hz while it moves, and on every other change). `clock` keeps
// the last push and advances it by wall time while playing, so anything drawn per frame (the
// playhead, readouts, karaoke lanes) is smooth at the display's rate, 60 or 120 Hz, while
// Svelte only re-renders on pushes. Per-frame work goes through ONE shared animation loop that
// runs only while something is subscribed.

import type { Action } from "svelte/action";
import type { TransportSnapshot } from "@workstation/engine-api";
import { transportSeconds } from "./state";

class TransportClock {
  #snapshot: TransportSnapshot = { transport: { kind: "Stopped" }, sample_rate: 44100 };
  #at = 0;

  update(s: TransportSnapshot): void {
    this.#snapshot = s;
    this.#at = performance.now();
  }

  get snapshot(): TransportSnapshot {
    return this.#snapshot;
  }

  /** Playhead seconds at `now` (a `performance.now()` time): the last push, advanced while playing. */
  seconds(now = performance.now()): number {
    const base = transportSeconds(this.#snapshot);
    return this.#snapshot.transport.kind === "Playing" ? base + Math.max(0, now - this.#at) / 1000 : base;
  }
}

export const clock = new TransportClock();

type FrameListener = (now: number) => void;
const listeners = new Set<FrameListener>();
let raf = 0;

function tick(now: number) {
  for (const f of listeners) f(now);
  raf = listeners.size > 0 ? requestAnimationFrame(tick) : 0;
}

/** Runs `f` once per displayed frame until the returned function is called. */
export function onFrame(f: FrameListener): () => void {
  listeners.add(f);
  if (raf === 0) raf = requestAnimationFrame(tick);
  return () => void listeners.delete(f);
}

/** Playhead x for a song time, or `null` when it's off the view. */
export type HeadX = (seconds: number) => number | null;

/**
 * Moves a playhead element to `toX(clock seconds)` every frame with a transform only, so the
 * compositor moves it and nothing is redrawn. Position the element at x = 0 of its lane.
 */
export const playhead: Action<HTMLElement, HeadX> = (node, initial) => {
  let toX = initial;
  let last: number | null | undefined;
  const stop = onFrame((now) => {
    const x = toX(clock.seconds(now));
    if (x === last) return;
    last = x;
    if (x === null) {
      node.style.visibility = "hidden";
    } else {
      node.style.visibility = "";
      node.style.transform = `translate3d(${x}px, 0, 0)`;
    }
  });
  return {
    update(next) {
      toX = next;
      last = undefined;
    },
    destroy: stop,
  };
};

/** Keeps the element's text at `format(clock seconds)`, written only when it changes. */
export const clockText: Action<HTMLElement, (seconds: number) => string> = (node, initial) => {
  let format = initial;
  let last = "";
  const stop = onFrame((now) => {
    const text = format(clock.seconds(now));
    if (text !== last) node.textContent = last = text;
  });
  return {
    update(next) {
      format = next;
      last = "";
    },
    destroy: stop,
  };
};
