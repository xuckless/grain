// The single home of every duration, easing and transition in the UI (AGENTS.md §8.5).
// Values come from the Grain design system's Motion board. Components import from here and
// never hard-code timing; stylesheets read the same values as CSS custom properties.

import type { Action } from "svelte/action";
import { crossfade, type TransitionConfig } from "svelte/transition";

/** Bezier control points of the three Grain curves. */
const curves = {
  out: [0.22, 1, 0.36, 1], // rule 1: entrances
  io: [0.65, 0, 0.35, 1], // rule 9: moves
  pop: [0.34, 1.56, 0.64, 1], // rule 13 exception: the score reveal only
} as const;
type Curve = readonly [number, number, number, number];

/** Solves a CSS cubic-bezier for y at time x (Newton steps, then bisection). */
function bezier([x1, y1, x2, y2]: Curve): (t: number) => number {
  const ax = 3 * x1 - 3 * x2 + 1, bx = 3 * x2 - 6 * x1, cx = 3 * x1;
  const ay = 3 * y1 - 3 * y2 + 1, by = 3 * y2 - 6 * y1, cy = 3 * y1;
  const sx = (u: number) => ((ax * u + bx) * u + cx) * u;
  const sy = (u: number) => ((ay * u + by) * u + cy) * u;
  return (x) => {
    if (x <= 0) return 0;
    if (x >= 1) return 1;
    let u = x;
    for (let i = 0; i < 6; i++) {
      const d = (3 * ax * u + 2 * bx) * u + cx;
      if (Math.abs(d) < 1e-6) break;
      u -= (sx(u) - x) / d;
    }
    let lo = 0, hi = 1;
    for (let i = 0; i < 20 && Math.abs(sx(u) - x) > 1e-5; i++) {
      if (sx(u) < x) lo = u;
      else hi = u;
      u = (lo + hi) / 2;
    }
    return sy(u);
  };
}

export const ease = {
  enter: bezier(curves.out),
  move: bezier(curves.io),
  celebrate: bezier(curves.pop),
} as const;

const cssCurve = (c: Curve) => `cubic-bezier(${c.join(", ")})`;

/** Entrance durations in ms; exits are always derived from these (rule 15). */
export const dur = {
  press: 120, // rule 4
  colour: 160, // hover / colour changes
  menu: 200, // rule 2: dropdowns and popovers
  enter: 280, // panels, rows, state layers
  detail: 240, // rule 6: details inside a panel
  iconSwap: 200, // rule 18
  tab: 280, // sliding tab indicator (rule 9)
  toggle: 240,
  wipe: 320, // rule 17: mask wipes
  reveal: 520, // canvas reveals (waveforms, score graph)
  pulse: 360, // rule 8
  celebrate: 560, // score reveal
  settle: 260, // knob stretch and rubber-band spring-back
  stagger: 30, // rule 3
  staggerCap: 270, // a long list's last row starts by 270 ms
  detailLead: 90, // rule 6: details start after their container
  toast: 3200, // how long a toast stays before dissolving
  dissolve: 1300, // a toast's grains drifting away
} as const;

/** Rule 15: exits are 20% shorter, always derived. */
export const EXIT_FACTOR = 0.8;
export const exitDuration = (enter: number) => enter * EXIT_FACTOR;

export const PRESS_SCALE = 0.97; // rule 4
export const POPOVER_SCALE = 0.95; // rule 10
export const KNOB_MAX_STRETCH = 0.12; // rule 5
export const PULSE_SCALE = 1.05; // rule 8
const ICON_SCALE = 0.7; // rule 18
const ICON_BLUR_PX = 3; // rule 18: 2–4 px
const ENTER_OFFSET_PX = 8;
const DETAIL_OFFSET_PX = 6;
const CELEBRATE_FROM = 0.55;

const reducedQuery = typeof matchMedia === "function" ? matchMedia("(prefers-reduced-motion: reduce)") : undefined;
export const reducedMotion = (): boolean => reducedQuery?.matches ?? false;

/**
 * Publishes the tokens as CSS custom properties on :root and keeps them in sync with
 * prefers-reduced-motion, which collapses every duration to 0.
 */
export function installMotionTokens(): () => void {
  const apply = () => {
    const s = document.documentElement.style;
    const reduced = reducedMotion();
    const ms = (v: number) => `${reduced ? 0 : v}ms`;
    s.setProperty("--ease-out", cssCurve(curves.out));
    s.setProperty("--ease-io", cssCurve(curves.io));
    s.setProperty("--dur-press", ms(dur.press));
    s.setProperty("--dur-colour", ms(dur.colour));
    s.setProperty("--dur-menu", ms(dur.menu));
    s.setProperty("--dur-menu-exit", ms(exitDuration(dur.menu)));
    s.setProperty("--dur-tab", ms(dur.tab));
    s.setProperty("--dur-toggle", ms(dur.toggle));
    s.setProperty("--dur-wipe", ms(dur.wipe));
    s.setProperty("--dur-detail", ms(dur.detail));
    s.setProperty("--press-scale", reduced ? "1" : String(PRESS_SCALE));
    s.setProperty("--popover-scale", reduced ? "1" : String(POPOVER_SCALE));
    s.setProperty("--knob-stretch-max", String(1 + KNOB_MAX_STRETCH));
    s.setProperty("--stagger", ms(dur.stagger));
  };
  apply();
  reducedQuery?.addEventListener("change", apply);
  return () => reducedQuery?.removeEventListener("change", apply);
}

/** Rules 3 + 6: delay of row `index` in a list whose container entered first. */
export function detailDelay(index: number): number {
  return dur.detailLead + Math.min(index * dur.stagger, dur.staggerCap);
}

type Direction = { direction: "in" | "out" | "both" };

/** Duration for a transition run in `direction` (exits derived, rule 15). */
function directed(enter: number, options?: Direction): number {
  return options?.direction === "out" ? exitDuration(enter) : enter;
}

/**
 * Entrance/exit for panels, rows and state layers: ease-out fade + rise (rules 1, 15).
 * State swaps stack old and new layers in one grid cell so they overlap (rule 16).
 */
export function enter(_node: Element, params: { delay?: number; detail?: boolean } = {}, options?: Direction): TransitionConfig {
  const out = options?.direction === "out";
  const offset = params.detail ? DETAIL_OFFSET_PX : ENTER_OFFSET_PX;
  return {
    delay: out ? 0 : (params.delay ?? 0),
    duration: directed(params.detail ? dur.detail : dur.enter, options),
    easing: ease.enter,
    css: reducedMotion() ? (t) => `opacity: ${t}` : (t, u) => `opacity: ${t}; transform: translateY(${u * offset}px)`,
  };
}

/**
 * Rules 10–12: popovers and menus start at 0.95 and grow out of, then shrink back into, their
 * trigger. The element's `transform-origin` (set by the caller) points at the trigger.
 */
export function popover(_node: Element, _params: object = {}, options?: Direction): TransitionConfig {
  return {
    duration: directed(dur.menu, options),
    easing: ease.enter,
    css: reducedMotion() ? (t) => `opacity: ${t}` : (t) => `opacity: ${t}; transform: scale(${POPOVER_SCALE + (1 - POPOVER_SCALE) * t})`,
  };
}

/** Rule 13 exception: the final score reveal is the one moment that may overshoot. */
export function celebrate(_node: Element, params: { delay?: number } = {}, options?: Direction): TransitionConfig {
  return {
    delay: params.delay ?? 0,
    duration: directed(dur.celebrate, options),
    easing: reducedMotion() ? ease.enter : ease.celebrate,
    css: reducedMotion()
      ? (t) => `opacity: ${t}`
      : (t) => `opacity: ${Math.min(1, t)}; transform: scale(${CELEBRATE_FROM + (1 - CELEBRATE_FROM) * t})`,
  };
}

/** Rule 18: icon swaps crossfade with a slight scale and blur. */
export function iconSwap(_node: Element, _params: object = {}, options?: Direction): TransitionConfig {
  return {
    duration: directed(dur.iconSwap, options),
    easing: ease.enter,
    css: reducedMotion()
      ? (t) => `opacity: ${t}`
      : (t, u) => `opacity: ${t}; transform: scale(${ICON_SCALE + (1 - ICON_SCALE) * t}); filter: blur(${u * ICON_BLUR_PX}px)`,
  };
}

/**
 * Rule 20: an element shown in two places (a recent song's title on Home, the song title in the
 * session top bar) moves between them as one element. Pair `send`/`receive` under one key.
 */
export const [sendShared, receiveShared] = crossfade({
  duration: dur.enter,
  easing: ease.move,
  fallback: (node, params, intro) => enter(node, params, { direction: intro ? "in" : "out" }),
});

/** Rule 8: one brief pulse confirming a completed action, whenever `trigger` changes. */
export const pulse: Action<HTMLElement, unknown> = (node, trigger) => {
  const run = () => {
    const frames = reducedMotion()
      ? [{ opacity: 1 }, { opacity: 0.7 }, { opacity: 1 }]
      : [{ transform: "scale(1)" }, { transform: `scale(${PULSE_SCALE})`, offset: 0.4 }, { transform: "scale(1)" }];
    node.animate(frames, { duration: dur.pulse, easing: cssCurve(curves.out) });
  };
  let last = trigger;
  return {
    update(next) {
      if (next === last) return;
      last = next;
      run();
    },
  };
};

/** Rule 17: clip-path wipe (left → right) when `trigger` is first set or changes.
 * `trigger` must be a primitive (a string or number key): Svelte deep-reads object action
 * parameters on every update, which costs milliseconds on a project or report (AGENTS.md §10.1).
 * For "replay when this object is replaced", pass `identity(obj)`. */
export const reveal: Action<HTMLElement, string | number | null> = (node, trigger) => {
  const run = () => {
    const frames = reducedMotion() ? [{ opacity: 0 }, { opacity: 1 }] : [{ clipPath: "inset(0 100% 0 0)" }, { clipPath: "inset(0 0 0 0)" }];
    node.animate(frames, { duration: dur.reveal, easing: cssCurve(curves.io) });
  };
  let last = trigger;
  run();
  return {
    update(next) {
      if (next !== last) {
        last = next;
        run();
      }
    },
  };
};

const identities = new WeakMap<object, number>();
let nextIdentity = 1;

/** A number unique to `o` (for primitive action keys, see `reveal`). */
export function identity(o: object | null): number {
  if (o === null) return 0;
  let id = identities.get(o);
  if (id === undefined) identities.set(o, (id = nextIdentity++));
  return id;
}

/** Rule 5: knob stretch for a pointer moving `velocityPx` per event, capped at 12%. */
export function knobStretch(velocityPx: number): number {
  return reducedMotion() ? 1 : 1 + Math.min(KNOB_MAX_STRETCH, Math.abs(velocityPx) * 0.012);
}

/** Rule 19: damped displacement for dragging `overshoot` px past a bound (max `limit` px). */
export function rubberBand(overshoot: number, limit: number): number {
  if (reducedMotion()) return 0;
  const sign = Math.sign(overshoot);
  const d = Math.abs(overshoot);
  return sign * limit * (1 - 1 / ((d * 0.55) / limit + 1));
}

/** Rules 19 + 9: animates `from` back to `to` with ease-in-out; returns a cancel function. */
export function springBack(from: number, onFrame: (value: number) => void, onDone: () => void, to = 0): () => void {
  if (reducedMotion() || from === to) {
    onFrame(to);
    onDone();
    return () => {};
  }
  const start = performance.now();
  let frame = 0;
  const step = (now: number) => {
    const t = Math.min(1, (now - start) / dur.settle);
    onFrame(from + (to - from) * ease.move(t));
    if (t < 1) frame = requestAnimationFrame(step);
    else onDone();
  };
  frame = requestAnimationFrame(step);
  return () => cancelAnimationFrame(frame);
}
