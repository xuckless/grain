// The session's shared timeline viewport (AGENTS.md §8.6): one zoom and scroll position for Mix,
// Score, Karaoke, Auto-tune, Pitch and Produce, so zooming into the chorus in one view keeps it
// in view in the next. Pure functions here; the session holds the value, `viewportGestures`
// drives it from the wheel and trackpad.

import type { Action } from "svelte/action";
import { rubberBand, springBack } from "./motion.ts";

/** Visible window of the song timeline, in seconds. `overshoot` is a transient rubber-band
 * offset (seconds) past either end while scrolling (rule 19); it springs back to 0. */
export type View = { start: number; span: number; overshoot: number };

/** The shortest visible span: a couple of bars at any tempo. */
export const MIN_SPAN_SECONDS = 2;
/** Rubber-band limit past either end, as a fraction of the visible span. */
const RUBBER_FRACTION = 0.06;

export const fitView = (duration: number): View => ({ start: 0, span: Math.max(MIN_SPAN_SECONDS, duration), overshoot: 0 });

/** Keeps `start` and `span` inside the song. */
export function clampView(v: View, duration: number): View {
  const span = Math.min(Math.max(MIN_SPAN_SECONDS, v.span), Math.max(MIN_SPAN_SECONDS, duration));
  const start = Math.min(Math.max(0, v.start), Math.max(0, duration - span));
  return { start, span, overshoot: v.overshoot };
}

/** Zooms by `factor` (< 1 zooms in) keeping the time under `anchor` (0–1 across the lane) still. */
export function zoomAt(v: View, factor: number, anchor: number, duration: number): View {
  const at = v.start + anchor * v.span;
  const span = v.span * factor;
  return clampView({ start: at - anchor * span, span, overshoot: 0 }, duration);
}

/** Pans by `seconds`; past either end the view gives with damped resistance (rule 19). */
export function panBy(v: View, seconds: number, duration: number): View {
  const wanted = v.start + v.overshoot + seconds;
  const clamped = clampView({ ...v, start: wanted, overshoot: 0 }, duration);
  const past = wanted - clamped.start;
  return { ...clamped, overshoot: rubberBand(past, v.span * RUBBER_FRACTION) };
}

/** x (px) of song time `seconds` in a lane `width` px wide. */
export const timeToX = (v: View, seconds: number, width: number): number => ((seconds - v.start - v.overshoot) / v.span) * width;

/** Song time at x (px) in a lane `width` px wide. */
export const xToTime = (v: View, x: number, width: number): number => v.start + v.overshoot + (x / width) * v.span;

/** Whether the whole song is visible. */
export const isFit = (v: View, duration: number): boolean => v.start <= 0 && v.span >= duration - 1e-6;

/** Wheel and pinch on a lane: ⌘/Ctrl-scroll or a trackpad pinch zooms around the pointer;
 * horizontal (or ⇧) scroll pans. Plain vertical scroll is left to the page. */
export const viewportGestures: Action<
  HTMLElement,
  { get: () => View; set: (v: View) => void; duration: number }
> = (node, initial) => {
  let p = initial;
  let settle: (() => void) | null = null;
  let idle = 0;
  const release = () => {
    const v = p.get();
    if (v.overshoot === 0) return;
    settle = springBack(
      v.overshoot,
      (o) => p.set({ ...p.get(), overshoot: o }),
      () => (settle = null),
    );
  };
  const wheel = (e: WheelEvent) => {
    const rect = node.getBoundingClientRect();
    const v = p.get();
    if (e.ctrlKey || e.metaKey) {
      e.preventDefault();
      settle?.();
      p.set(zoomAt(v, Math.exp(e.deltaY * 0.01), (e.clientX - rect.left) / rect.width, p.duration));
      return;
    }
    const dx = e.shiftKey ? e.deltaY : e.deltaX;
    if (dx === 0 || Math.abs(dx) < Math.abs(e.deltaY) * (e.shiftKey ? 0 : 1)) return;
    e.preventDefault();
    settle?.();
    p.set(panBy(v, (dx / rect.width) * v.span, p.duration));
    clearTimeout(idle);
    idle = window.setTimeout(release, 120);
  };
  node.addEventListener("wheel", wheel, { passive: false });
  return {
    update: (next) => (p = next),
    destroy: () => {
      node.removeEventListener("wheel", wheel);
      clearTimeout(idle);
      settle?.();
    },
  };
};
