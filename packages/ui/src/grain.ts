// Grain: the one material the interface is made of. Every canvas in the UI draws dots through
// this module: the logo mark, the particle behaviours from the design's Particles board, dot
// waveforms and dotted note bars. Ported from the Grain design system (Logo, Particles,
// Components boards); geometry that was drawn on fixed tiles is scaled to the canvas here.

import type { Action } from "svelte/action";
import type { Note } from "@workstation/engine-api";
import type { Feedback } from "./feedback";
import { colour, feedback, spectrum, stemHue } from "./theme";
import { onFrame } from "./clock";
import { dur, reducedMotion } from "./motion";

export type Ctx = CanvasRenderingContext2D;

/** One animated drawing, redrawn every frame on a cleared canvas sized `w`×`h` CSS px. */
export interface Behaviour {
  draw(c: Ctx, w: number, h: number, t: number, now: number): void;
  /** Whether it has nothing to show right now (the canvas is left cleared, no frames drawn). */
  idle?(now: number): boolean;
}

export function dot(c: Ctx, x: number, y: number, r: number, col: string): void {
  c.fillStyle = col;
  c.beginPath();
  c.arc(x, y, r, 0, Math.PI * 2);
  c.fill();
}

/** Adds a dot to the current path; fill many in one go (`c.beginPath()` … `c.fill()`): one
 * fill per colour instead of per dot is most of a busy canvas's frame time. */
export function dotPath(c: Ctx, x: number, y: number, r: number): void {
  c.moveTo(x + r, y);
  c.arc(x, y, r, 0, Math.PI * 2);
}

/** A horizontal bar of dots from `x0` to `x1` centred on `y` (`rows` dots tall). */
export function dotBar(c: Ctx, x0: number, x1: number, y: number, col: string, r = 0.9, rows = 3, step = 5): void {
  for (let x = x0; x <= x1; x += step) for (let k = 0; k < rows; k++) dot(c, x, y + (k - (rows - 1) / 2) * 4, r, col);
}

/** A row of dots across the canvas lit up to `fraction` (0–1): download progress, meters.
 * `lit` defaults to the text colour. */
export const dotFill: Action<HTMLCanvasElement, number | { fraction: number; lit: string }> = (canvas, value) => {
  const draw = (v: number | { fraction: number; lit: string }) => {
    const { fraction, lit } = typeof v === "number" ? { fraction: v, lit: colour.text } : v;
    dotRow(canvas, fraction, lit);
  };
  draw(value);
  return { update: draw };
};

/** Draws a row of dots across `canvas` (8 px tall) lit up to `fraction`. */
export function dotRow(canvas: HTMLCanvasElement, fraction: number, lit: string = colour.text): void {
  const w = canvas.clientWidth, h = 8;
  const c = fit(canvas, w, h);
  if (!c) return;
  for (let x = 3; x <= w - 3; x += 6) dot(c, x, h / 2, 1.5, x / w <= fraction ? lit : colour.raised);
}

/** A horizontal input meter: `level` (0–1 peak) shown on a dB scale from −48 dB, lit dots
 * over a track of dim ones. */
export function dotMeter(c: Ctx, w: number, h: number, level: number, lit: string, track: string): void {
  const db = level > 0 ? 20 * Math.log10(level) : -Infinity;
  const fill = Math.max(0, Math.min(1, (db + 48) / 48));
  const step = 6;
  for (let x = 3; x <= w - 3; x += step) dot(c, x, h / 2, 1.6, x / w <= fill ? lit : track);
}

export function fit(canvas: HTMLCanvasElement, w: number, h: number): Ctx | null {
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  const bw = Math.max(1, Math.round(w * dpr)), bh = Math.max(1, Math.round(h * dpr));
  if (canvas.width !== bw || canvas.height !== bh) {
    canvas.width = bw;
    canvas.height = bh;
  }
  const c = canvas.getContext("2d");
  c?.setTransform(dpr, 0, 0, dpr, 0, 0);
  c?.clearRect(0, 0, w, h);
  return c;
}

/**
 * Runs `behaviour` on the canvas every animation frame at the canvas's CSS size.
 * Reduced motion freezes time (`t` = 0), so fields hold still.
 */
export const grain: Action<HTMLCanvasElement, Behaviour> = (canvas, initial) => {
  let behaviour = initial;
  /** Off screen (scrolled away, or a hidden view): no frames are drawn (AGENTS.md §10.1). */
  let visible = true;
  let cleared = false;
  const seen = new IntersectionObserver((entries) => {
    for (const e of entries) visible = e.isIntersecting;
  });
  seen.observe(canvas);
  const stop = onFrame((ts) => {
    if (!visible) return;
    const now = performance.now();
    const w = canvas.clientWidth, h = canvas.clientHeight;
    if (behaviour.idle?.(now)) {
      if (!cleared) fit(canvas, w, h);
      cleared = true;
      return;
    }
    cleared = false;
    const c = w > 0 && h > 0 ? fit(canvas, w, h) : null;
    if (c) behaviour.draw(c, w, h, reducedMotion() ? 0 : ts / 1000, now);
  });
  return {
    update(next) {
      behaviour = next;
    },
    destroy() {
      stop();
      seen.disconnect();
    },
  };
};

const rand = (lo: number, hi: number) => lo + Math.random() * (hi - lo);

// ---------------------------------------------------------------------------------------------
// Logo mark: a round field of dots rippling like sound (design: Logo board).

/** Size the mark was designed at; `spacing` and ripple speed scale from it. */
const ORB_DESIGN_SIZE = 600;

export class Orb implements Behaviour {
  private ripples: { x: number; y: number; t0: number }[] = [];
  /** `spacing`: dot grid step in px (9 at 600 px; ~2.6 for a 44 px mark). */
  constructor(private readonly spacing: number) {}

  /** Sends a ripple out from (`x`, `y`) in canvas px. */
  ripple(x: number, y: number): void {
    if (reducedMotion()) return;
    this.ripples.push({ x, y, t0: performance.now() });
    if (this.ripples.length > 4) this.ripples.shift();
  }

  draw(c: Ctx, w: number, h: number, t: number, now: number): void {
    const W = Math.min(w, h), s = this.spacing;
    const cx = w / 2, cy = h / 2, R = W * 0.42, k = s / 9;
    const scale = W / ORB_DESIGN_SIZE;
    this.ripples = this.ripples.filter((r) => now - r.t0 < 1800);
    for (let gy = cy - R; gy <= cy + R; gy += s) {
      for (let gx = cx - R; gx <= cx + R; gx += s) {
        const dx = gx - cx, dy = gy - cy, r = Math.hypot(dx, dy);
        if (r > R) continue;
        const th = Math.atan2(dy, dx);
        const edge = R * (0.86 + 0.07 * Math.sin(4 * th + t * 0.5) + 0.04 * Math.sin(7 * th - t * 0.8));
        if (r > edge) continue;
        const rn = r / k;
        let wave = 0.5 + 0.5 * Math.sin(rn * 0.055 - t * 2.0);
        let px = gx, py = gy;
        const push = 2.5 * k * Math.sin(rn * 0.05 - t * 2.0);
        if (r > 0) {
          px += (dx / r) * push;
          py += (dy / r) * push;
        }
        for (const rp of this.ripples) {
          const age = (now - rp.t0) / 1000;
          const front = age * 320 * scale;
          const dd = Math.hypot(gx - rp.x, gy - rp.y);
          const band = Math.exp(-((dd - front) ** 2) / (648 * scale * scale)) * (1 - age / 1.8);
          if (band > 0.01) {
            wave += band * 1.3;
            if (dd > 0) {
              px += ((gx - rp.x) / dd) * band * 9 * scale;
              py += ((gy - rp.y) / dd) * band * 9 * scale;
            }
          }
        }
        const soft = 1 - Math.max(0, (r - (edge - 36 * k)) / (36 * k)) * 0.55;
        const dia = Math.min(4.4 * k, (0.9 + 2.5 * wave) * k * soft);
        dot(c, px, py, Math.max(0.35, dia / 2), colour.text);
      }
    }
  }
}

// ---------------------------------------------------------------------------------------------
// Breath: idle and listening. `intensity` 0 = calm, 1 = listening (edges ripple in vocal pink).

export class Breath implements Behaviour {
  intensity = 0;
  private level = 0;

  draw(c: Ctx, w: number, h: number, t: number): void {
    const target = this.intensity > 0 ? this.intensity : 0.15 + 0.1 * Math.sin(t * 0.8);
    this.level += (target - this.level) * 0.05;
    const L = this.level;
    const cx = w / 2, cy = h / 2, R = Math.min(w, h) * 0.39, s = Math.max(5, R / 15);
    for (let y = cy - R; y <= cy + R; y += s) {
      for (let x = cx - R; x <= cx + R; x += s) {
        const dx = x - cx, dy = y - cy, r = Math.hypot(dx, dy), th = Math.atan2(dy, dx);
        const edge = R * (0.8 + L * (0.08 * Math.sin(5 * th + t * 2.2) + 0.05 * Math.sin(9 * th - t * 3)) + 0.03 * Math.sin(3 * th + t * 0.6));
        if (r > edge) continue;
        const wv = 0.5 + 0.5 * Math.sin((r / R) * 8.3 - t * (1.2 + L * 3));
        const dia = (0.8 + wv * (1.4 + L * 2.2)) * (1 - Math.max(0, (r - edge + 26) / 26) * 0.5);
        const col = L > 0.4 && r > edge - 22 ? stemHue.Vocals : colour.text;
        dot(c, x, y, Math.max(0.4, Math.min(3.6, dia) / 2), col);
      }
    }
  }
}

// ---------------------------------------------------------------------------------------------
// Terrain: landing and empty states. Layered dot ridges in stem hues, with drips.

const TERRAIN_DESIGN_H = 300;

export class Terrain implements Behaviour {
  private drips: { x: number; y: number; vy: number; c: string; len: number }[] = [];

  draw(c: Ctx, w: number, h: number, t: number): void {
    const sy = h / TERRAIN_DESIGN_H;
    const layers = [
      { base: 92, amp: 34, f: 0.012, sp: 0.25, col: stemHue.Bass },
      { base: 112, amp: 30, f: 0.017, sp: -0.32, col: stemHue.Vocals },
      { base: 128, amp: 24, f: 0.022, sp: 0.4, col: stemHue.Drums },
      { base: 146, amp: 18, f: 0.03, sp: -0.2, col: stemHue.Guitar },
    ];
    const columns = Math.ceil(w / 4) + 1;
    const lowest = new Array<number>(columns).fill(0);
    layers.forEach((L, li) => {
      c.fillStyle = L.col;
      c.beginPath();
      for (let i = 0; i < columns; i++) {
        const x = i * 4;
        const y = sy * (L.base + L.amp * Math.sin(x * L.f + t * L.sp + li) * Math.cos(x * L.f * 0.43 - t * L.sp * 0.7) + 6 * Math.sin(x * 0.09 + t + li));
        const thick = 5 + Math.round(3 + 3 * Math.sin(x * 0.05 + li + t * 0.3));
        for (let k = 0; k < thick; k++) dotPath(c, x + (k % 2) * 2, y + k * 3.6 * sy, 1.25 - (k / thick) * 0.6);
        lowest[i] = Math.max(lowest[i], y + thick * 3.6 * sy);
      }
      c.fill();
    });
    if (!reducedMotion() && Math.random() < 0.5 * (w / 405)) {
      const i = Math.floor(Math.random() * columns);
      const cols = [stemHue.Vocals, stemHue.Drums, stemHue.Bass, colour.text];
      this.drips.push({ x: i * 4, y: lowest[i], vy: rand(0.6, 2), c: cols[Math.floor(Math.random() * cols.length)], len: 2 + Math.floor(Math.random() * 5) });
    }
    for (const d of this.drips) d.y += d.vy;
    this.drips = this.drips.filter((d) => d.y < h + 20);
    for (const d of this.drips) for (let k = 0; k < d.len; k++) dot(c, d.x, d.y - k * 4, 0.9 - k * 0.1, d.c);
  }
}

// ---------------------------------------------------------------------------------------------
// Gather: work in progress. Grains drift scattered, then gather into a spinning ring as
// `progress` (0–1) rises: grain i joins the ring once progress passes i / count.

const GATHER_COUNT = 96;

export class Gather implements Behaviour {
  progress = 0;
  private parts: { x: number; y: number; hx: number; hy: number; a: number; c: string }[] = [];

  draw(c: Ctx, w: number, h: number, t: number): void {
    if (this.parts.length === 0) {
      for (let i = 0; i < GATHER_COUNT; i++) {
        const hx = Math.random() * w, hy = Math.random() * h;
        this.parts.push({ x: hx, y: hy, hx, hy, a: (i / GATHER_COUNT) * Math.PI * 2, c: spectrum[i % spectrum.length] });
      }
    }
    const cx = w / 2, cy = h / 2, ring = Math.min(w, h) * 0.3;
    this.parts.forEach((p, i) => {
      const joined = i / GATHER_COUNT < this.progress;
      let tx: number, ty: number, r: number, col: string;
      if (joined) {
        const rr = ring + 4 * Math.sin(p.a * 6 + t * 3);
        tx = cx + Math.cos(p.a + t * 1.4) * rr;
        ty = cy + Math.sin(p.a + t * 1.4) * rr;
        r = 1.5;
        col = p.c;
      } else {
        tx = p.hx + Math.sin(t + p.a * 3) * 6;
        ty = p.hy + Math.cos(t * 0.8 + p.a * 2) * 6;
        r = 1;
        col = colour.muted;
      }
      const ease = joined ? 0.12 : 0.04;
      p.x += (tx - p.x) * ease;
      p.y += (ty - p.y) * ease;
      dot(c, p.x, p.y, r, col);
    });
  }
}

// ---------------------------------------------------------------------------------------------
// Dot progress bar (design: Components → Separation). Four rows of stem hues fill left to right.

export class DotProgress implements Behaviour {
  progress = 0;

  draw(c: Ctx, w: number, h: number, _t: number, now: number): void {
    const rows = 4, cols = Math.max(8, Math.floor(w / 8));
    const sx = w / cols, sy = h / rows;
    for (let j = 0; j < rows; j++) {
      for (let i = 0; i < cols; i++) {
        const filled = (i + 0.5) / cols <= this.progress;
        let x = i * sx + sx / 2, y = j * sy + sy / 2;
        if (filled) {
          dot(c, x, y, 2.3, spectrum[j]);
        } else {
          const jit = reducedMotion() ? 0 : Math.sin(now * 0.004 + i * 1.7 + j * 2.3) * 1.5;
          x += jit;
          y += jit * 0.6;
          dot(c, x, y, 1, colour.muted);
        }
      }
    }
  }
}

// ---------------------------------------------------------------------------------------------
// Hit burst: confirms a success (stems ready, save confirmed, score revealed). Drawn on one
// full-window overlay; `fire` takes viewport coordinates.

const BURST_GRAINS = 46;
const BURST_LIFE_MS = 3200;
const RING_MS = 420;

export class Burst implements Behaviour {
  private parts: { x: number; y: number; vx: number; vy: number; r: number; c: string; born: number }[] = [];
  private rings: { x: number; y: number; t0: number }[] = [];

  fire(x: number, y: number): void {
    const now = performance.now();
    this.rings.push({ x, y, t0: now });
    if (reducedMotion()) return;
    const hues = [...spectrum, colour.text];
    for (let i = 0; i < BURST_GRAINS; i++) {
      const a = Math.random() * Math.PI * 2, sp = rand(1, 4.6);
      this.parts.push({ x, y, vx: Math.cos(a) * sp, vy: Math.sin(a) * sp - 1.6, r: rand(1.2, 2.8), c: hues[i % hues.length], born: now });
    }
  }

  /** Fires from the centre of `el`. */
  fireAt(el: Element): void {
    const r = el.getBoundingClientRect();
    this.fire(r.left + r.width / 2, r.top + r.height / 2);
  }

  idle(): boolean {
    return this.rings.length === 0 && this.parts.length === 0;
  }

  draw(c: Ctx, _w: number, h: number, _t: number, now: number): void {
    this.rings = this.rings.filter((g) => now - g.t0 <= RING_MS);
    for (const g of this.rings) {
      const a = (now - g.t0) / RING_MS;
      for (let i = 0; i < 24; i++) {
        const th = (i / 24) * Math.PI * 2;
        dot(c, g.x + Math.cos(th) * (8 + a * 34), g.y + Math.sin(th) * (8 + a * 34), 1.6 * (1 - a), colour.text);
      }
    }
    for (const p of this.parts) {
      p.vy += 0.09;
      p.vx *= 0.985;
      p.x += p.vx;
      p.y += p.vy;
    }
    this.parts = this.parts.filter((p) => p.y < h + 6 && now - p.born < BURST_LIFE_MS);
    for (const p of this.parts) {
      const fade = Math.max(0, 1 - (now - p.born) / BURST_LIFE_MS);
      dot(c, p.x, p.y, p.r * fade + 0.3, p.c);
      if (p.vy > 2.2) dot(c, p.x, p.y - 4, (p.r * fade + 0.3) * 0.6, p.c);
    }
  }
}

// ---------------------------------------------------------------------------------------------
// Dissolve: a toast leaving. Its text, rebuilt as grains, drifts off left to right.

type DissolvePhase = { kind: "shown" } | { kind: "dissolving"; since: number };

export class Dissolve implements Behaviour {
  private parts: { tx: number; ty: number; x: number; y: number; vx: number; vy: number; delay: number; c: string }[] = [];
  private phase: DissolvePhase = { kind: "shown" };
  private built = "";

  constructor(
    private readonly text: string,
    private readonly font: string,
  ) {}

  start(): void {
    if (this.phase.kind === "dissolving") return;
    this.phase = { kind: "dissolving", since: performance.now() };
    for (const p of this.parts) {
      p.vx = rand(0.4, 2.6);
      p.vy = rand(-1.8, -0.6);
    }
  }

  private build(w: number, h: number): void {
    const off = document.createElement("canvas");
    off.width = Math.ceil(w);
    off.height = Math.ceil(h);
    const o = off.getContext("2d");
    if (!o) return;
    o.fillStyle = "#fff";
    o.font = this.font;
    o.textAlign = "center";
    o.textBaseline = "middle";
    o.fillText(this.text, w / 2, h / 2);
    const d = o.getImageData(0, 0, off.width, off.height).data;
    this.parts = [];
    for (let y = 0; y < off.height; y += 2) {
      for (let x = 0; x < off.width; x += 2) {
        if (d[(y * off.width + x) * 4 + 3] > 150) {
          this.parts.push({ tx: x, ty: y, x, y, vx: 0, vy: 0, delay: (x / w) * 380, c: spectrum[Math.floor((x / w) * spectrum.length)] });
        }
      }
    }
    this.built = `${w}x${h}`;
  }

  draw(c: Ctx, w: number, h: number, _t: number, now: number): void {
    if (this.built !== `${w}x${h}`) this.build(w, h);
    const phase = this.phase;
    for (const p of this.parts) {
      if (phase.kind === "shown") {
        dot(c, p.tx, p.ty, 0.9, colour.text);
        continue;
      }
      const age = now - phase.since;
      const moved = age > p.delay;
      if (moved) {
        p.vy += 0.05;
        p.x += p.vx;
        p.y += p.vy;
      }
      const r = moved ? 0.9 * Math.max(0.2, 1 - (age - p.delay) / (dur.dissolve - 300)) : 0.9;
      dot(c, p.x, p.y, r, moved ? p.c : colour.text);
    }
  }
}

// ---------------------------------------------------------------------------------------------
// Pitch trail: the karaoke lane (design: Particles → Pitch trail). Reference notes scroll right
// to left as dotted bars past the playhead; each sung pitch leaves grains at its song time,
// coloured hit / near / miss, which scroll with the notes and fade. Notes judged by a live
// performance fill with their verdict. The y-axis is semitones, ranged to the song's notes.

const TRAIL_PX_PER_SECOND = 110;
const TRAIL_LIFE_S = 2.6;
/** Semitones of headroom above and below the song's range. */
const TRAIL_MARGIN = 2;

/** A sung pitch at a song time. `feedback`: its verdict, `null` when there's nothing to sing. */
export type TrailPoint = { seconds: number; midi: number; feedback: Feedback | null };

export class PitchTrail implements Behaviour {
  private parts: { seconds: number; born: number; y: number; vy: number; r: number; c: string }[] = [];
  private queued: TrailPoint[] = [];
  private last: TrailPoint | null = null;
  private song = 0;
  private readonly lo: number;
  private readonly hi: number;
  private geometry = { headX: 0, y: (_midi: number) => 0 };
  /** Verdicts of judged notes (index-aligned with `notes`), drawn as filled bars. */
  fills: readonly (Feedback | null)[] = [];

  /**
   * `clock(now)`: the song time (seconds) at animation time `now` (ms). `head`: playhead
   * position as a fraction of the width. `sample(seconds)`: a point to draw at each frame (the
   * Score preview, which plays the reference melody); live performances `add` points instead.
   */
  constructor(
    private readonly notes: readonly Note[],
    private readonly clock: (now: number) => number,
    private readonly head = 0.7,
    private readonly sample?: (seconds: number) => TrailPoint | null,
  ) {
    let lo = Infinity, hi = -Infinity;
    for (const n of notes) {
      lo = Math.min(lo, n.midi);
      hi = Math.max(hi, n.midi);
    }
    this.lo = lo - TRAIL_MARGIN;
    this.hi = Math.max(hi + TRAIL_MARGIN, this.lo + 2 * TRAIL_MARGIN);
  }

  /** Queues sung points for the next frame. */
  add(points: readonly TrailPoint[]): void {
    this.queued.push(...points);
  }

  /** Forgets the trail (a new performance). */
  clear(): void {
    this.parts = [];
    this.queued = [];
    this.last = null;
  }

  /** Canvas position of the playhead at the last sung pitch (or mid-height), for bursts. */
  headPoint(): { x: number; y: number } {
    const { headX, y } = this.geometry;
    return { x: headX, y: y(this.last?.midi ?? (this.lo + this.hi) / 2) };
  }

  draw(c: Ctx, w: number, h: number, _t: number, now: number): void {
    if (this.notes.length === 0) return;
    const song = this.clock(now);
    // A seek backwards leaves grains in the future: start the trail afresh.
    if (song < this.song - 0.25) this.clear();
    this.song = song;
    const { lo, hi } = this;
    const y = (midi: number) => h - 12 - ((Math.min(hi, Math.max(lo, midi)) - lo) / (hi - lo)) * (h - 24);
    const headX = w * this.head;
    const x = (seconds: number) => headX + (seconds - song) * TRAIL_PX_PER_SECOND;
    this.geometry = { headX, y };

    // Octave guides (every C), faint.
    for (let m = Math.ceil(lo); m <= hi; m++) if (m % 12 === 0) for (let gx = 0; gx < w; gx += 8) dot(c, gx, y(m), 0.5, colour.disabled);
    // The playhead.
    for (let gy = 6; gy < h - 6; gy += 6) dot(c, headX, gy, 0.6, colour.disabled);

    this.notes.forEach((n, i) => {
      const x0 = x(n.start), x1 = x(n.end);
      if (x1 < 0 || x0 > w) return;
      const fill = this.fills[i];
      dotBar(c, Math.max(0, x0), Math.min(w, x1), y(n.midi), fill ? feedback[fill] : colour.muted, fill ? 1.1 : 0.9);
    });

    const sampled = this.sample?.(song);
    if (sampled) this.queued.push(sampled);
    for (const p of this.queued) {
      const col = p.feedback ? feedback[p.feedback] : colour.muted;
      if (!reducedMotion()) {
        for (let i = 0; i < 2; i++) this.parts.push({ seconds: p.seconds, born: now, y: y(p.midi) + rand(-1.5, 1.5), vy: rand(-0.12, 0.12), r: p.feedback === "hit" ? 2.3 : 1.7, c: col });
      }
      this.last = p;
    }
    this.queued = [];

    this.parts = this.parts.filter((p) => now - p.born < TRAIL_LIFE_S * 1000);
    for (const p of this.parts) {
      const age = (now - p.born) / 1000;
      const life = 1 - age / TRAIL_LIFE_S;
      dot(c, x(p.seconds), p.y + p.vy * age * 60, Math.max(0.3, p.r * life), p.c);
    }
    // The head: the latest sung pitch, while it's current.
    const l = this.last;
    if (l && song - l.seconds < 0.25) dot(c, x(l.seconds), y(l.midi), 4.5, l.feedback ? feedback[l.feedback] : colour.muted);
  }
}

// ---------------------------------------------------------------------------------------------
// Dot waveforms (design: Components → Tracks).

/** Width of one waveform column of dots. */
export const WAVE_STEP_PX = 5;

/**
 * Per-column peak level (0–1) of a track's `[min, max]` peak pairs, for `columns` columns
 * spanning the part of the song from `from` to `to` (fractions of the song; the shared timeline
 * viewport, AGENTS.md §8.6).
 */
export function columnLevels(peaks: readonly number[], columns: number, from = 0, to = 1): Float32Array {
  const pairs = peaks.length / 2;
  const out = new Float32Array(columns);
  for (let col = 0; col < columns; col++) {
    const a = Math.floor((from + (col / columns) * (to - from)) * pairs);
    const b = Math.max(a + 1, Math.floor((from + ((col + 1) / columns) * (to - from)) * pairs));
    let level = 0;
    for (let i = Math.max(0, a); i < Math.min(b, pairs); i++) level = Math.max(level, Math.abs(peaks[2 * i]), Math.abs(peaks[2 * i + 1]));
    out[col] = level;
  }
  return out;
}

const levelCache = new WeakMap<readonly number[], { columns: number; from: number; to: number; levels: Float32Array }>();

/** [`columnLevels`], memoised per peaks array (the same array until the song changes). */
export function cachedLevels(peaks: readonly number[], columns: number, from = 0, to = 1): Float32Array {
  const hit = levelCache.get(peaks);
  if (hit && hit.columns === columns && hit.from === from && hit.to === to) return hit.levels;
  const levels = columnLevels(peaks, columns, from, to);
  levelCache.set(peaks, { columns, from, to, levels });
  return levels;
}

/** Dot waveform: each column is a vertical run of dots whose count follows its level. */
export function drawDotWave(c: Ctx, w: number, h: number, levels: Float32Array, gain: number, col: string): void {
  const mid = h / 2, maxDots = Math.max(1, Math.floor((h / 2 - 3) / 4.6));
  c.fillStyle = col;
  c.beginPath();
  for (let i = 0; i < levels.length; i++) {
    const x = i * WAVE_STEP_PX + WAVE_STEP_PX / 2;
    if (x > w) break;
    const amp = Math.min(1, levels[i] * gain * 1.4);
    const n = Math.max(0, Math.round(amp * maxDots));
    for (let k = -n; k <= n; k++) dotPath(c, x, mid + k * 4.6, 1.25 - (Math.abs(k) / (n + 1)) * 0.35);
  }
  c.fill();
}
