// The single home of every duration, easing and transition in the UI (AGENTS.md §8.5).
// Components import from here and never hard-code timing.

import { backOut, cubicInOut, cubicOut } from "svelte/easing";
import type { Action } from "svelte/action";
import type { TransitionConfig } from "svelte/transition";

/** Durations in ms. */
export const dur = {
  press: 100,
  menu: 200, // rule 2: dropdowns/popovers 150–250 ms
  enter: 240,
  iconSwap: 180,
  pulse: 320,
  reveal: 520,
  settle: 260, // knob stretch and rubber-band spring-back
  stagger: 30, // rule 3
  staggerCap: 270, // last row starts by 270 ms, so a long list's stagger stays under 300 ms
  detailLead: 90, // rule 6: details start after their container
} as const;

/** Rule 15: exits are 20% shorter, always derived. */
export const EXIT_FACTOR = 0.8;
export const exitDuration = (enter: number) => enter * EXIT_FACTOR;

export const ease = {
  enter: cubicOut, // rule 1
  move: cubicInOut, // rule 9
  celebrate: backOut, // rule 13: overshoot only for rare celebratory moments
} as const;

/** The same curves for CSS and the Web Animations API. */
const cssEase = {
  enter: "cubic-bezier(0.33, 1, 0.68, 1)",
  move: "cubic-bezier(0.65, 0, 0.35, 1)",
} as const;

export const PRESS_SCALE = 0.97; // rule 4
export const KNOB_MAX_STRETCH = 0.12; // rule 5
const ICON_BLUR_PX = 3; // rule 18: 2–4 px
const ICON_SCALE = 0.85;
const ENTER_OFFSET_PX = 6;
const PULSE_SCALE = 1.04;

const reducedQuery = typeof matchMedia === "function" ? matchMedia("(prefers-reduced-motion: reduce)") : undefined;
export const reducedMotion = (): boolean => reducedQuery?.matches ?? false;

/**
 * Publishes the tokens as CSS custom properties on :root so stylesheets use the same values,
 * and keeps them in sync with prefers-reduced-motion.
 */
export function installMotionTokens(): () => void {
  const apply = () => {
    const s = document.documentElement.style;
    const reduced = reducedMotion();
    s.setProperty("--motion-press-dur", `${dur.press}ms`);
    s.setProperty("--motion-press-scale", reduced ? "1" : String(PRESS_SCALE));
    s.setProperty("--motion-wipe-dur", `${reduced ? 0 : dur.settle}ms`);
    s.setProperty("--motion-ease-enter", cssEase.enter);
    s.setProperty("--motion-ease-move", cssEase.move);
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
 * Entrance/exit for panels, rows and state layers: ease-out fade + small rise (rules 1, 15).
 * State swaps stack old and new layers in one grid cell so they overlap (rule 16).
 */
export function enter(_node: Element, params: { delay?: number } = {}, options?: Direction): TransitionConfig {
  const out = options?.direction === "out";
  return {
    delay: out ? 0 : (params.delay ?? 0),
    duration: directed(dur.enter, options),
    easing: ease.enter,
    css: reducedMotion() ? (t) => `opacity: ${t}` : (t, u) => `opacity: ${t}; transform: translateY(${u * ENTER_OFFSET_PX}px)`,
  };
}

/** Rule 13 exception: the final score reveal may overshoot. */
export function celebrate(_node: Element, params: { delay?: number } = {}, options?: Direction): TransitionConfig {
  return {
    delay: params.delay ?? 0,
    duration: directed(dur.pulse, options),
    easing: reducedMotion() ? ease.enter : ease.celebrate,
    css: reducedMotion() ? (t) => `opacity: ${t}` : (t) => `opacity: ${Math.min(1, t)}; transform: scale(${0.9 + 0.1 * t})`,
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
 * Rule 8: one brief pulse confirming a completed action: when the element appears (after its
 * entrance has finished) and again whenever `trigger` changes.
 */
export const pulse: Action<HTMLElement, unknown> = (node, trigger) => {
  const run = (delay: number) => {
    const frames = reducedMotion()
      ? [{ opacity: 1 }, { opacity: 0.7 }, { opacity: 1 }]
      : [{ transform: "scale(1)" }, { transform: `scale(${PULSE_SCALE})` }, { transform: "scale(1)" }];
    node.animate(frames, { duration: dur.pulse, delay, easing: cssEase.move });
  };
  let last = trigger;
  run(dur.detailLead + Math.max(dur.enter, dur.pulse));
  return {
    update(next) {
      if (next === last) return;
      last = next;
      run(0);
    },
  };
};

/** Rule 17: clip-path wipe (left → right) when `trigger` is first set or changes. */
export const reveal: Action<HTMLElement, unknown> = (node, trigger) => {
  const run = () => {
    const frames = reducedMotion()
      ? [{ opacity: 0 }, { opacity: 1 }]
      : [{ clipPath: "inset(0 100% 0 0)" }, { clipPath: "inset(0 0 0 0)" }];
    node.animate(frames, { duration: dur.reveal, easing: cssEase.enter });
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

/**
 * Rule 5: a range input's thumb stretches (≤ 12%) along its travel while it moves and settles
 * back afterwards. The thumb reads `--knob-stretch` (see the slider styles in App.svelte).
 * Rule 14: the value itself follows the pointer natively; only the thumb shape animates.
 */
export const knobStretch: Action<HTMLInputElement> = (node) => {
  let lastValue = Number(node.value);
  let lastTime = performance.now();
  let settle: ReturnType<typeof setTimeout> | undefined;
  const range = Number(node.max) - Number(node.min) || 1;
  const onInput = () => {
    if (reducedMotion()) return;
    const now = performance.now();
    const speed = Math.abs(Number(node.value) - lastValue) / range / Math.max(1, now - lastTime); // fraction per ms
    lastValue = Number(node.value);
    lastTime = now;
    const stretch = 1 + Math.min(KNOB_MAX_STRETCH, speed * 40);
    node.style.setProperty("--knob-stretch", stretch.toFixed(3));
    clearTimeout(settle);
    settle = setTimeout(() => node.style.setProperty("--knob-stretch", "1"), dur.settle / 2);
  };
  node.style.setProperty("--knob-stretch", "1");
  node.style.setProperty("--knob-settle", `${dur.settle}ms ${cssEase.move}`);
  node.addEventListener("input", onInput);
  return {
    destroy() {
      clearTimeout(settle);
      node.removeEventListener("input", onInput);
    },
  };
};

/** Rule 19: damped displacement for dragging `overshoot` px past a bound (max `limit` px). */
export function rubberBand(overshoot: number, limit: number): number {
  if (reducedMotion()) return 0;
  const sign = Math.sign(overshoot);
  const d = Math.abs(overshoot);
  return sign * limit * (1 - 1 / ((d * 0.55) / limit + 1));
}

/** Rule 19 + 9: animates a displacement back to 0 with ease-in-out; returns a cancel function. */
export function springBack(from: number, onFrame: (value: number) => void, onDone: () => void): () => void {
  if (reducedMotion() || from === 0) {
    onFrame(0);
    onDone();
    return () => {};
  }
  const start = performance.now();
  let frame = 0;
  const step = (now: number) => {
    const t = Math.min(1, (now - start) / dur.settle);
    onFrame(from * (1 - ease.move(t)));
    if (t < 1) frame = requestAnimationFrame(step);
    else onDone();
  };
  frame = requestAnimationFrame(step);
  return () => cancelAnimationFrame(frame);
}
