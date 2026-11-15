// The rotary knob's mapping (AGENTS.md §8.6, Mix): pure, unit-tested under Node. A knob turns
// through 270° (7 o'clock to 5 o'clock); dragging up turns it clockwise.

/** Sweep from the bottom-left stop to the bottom-right stop, in degrees. */
export const SWEEP_DEGREES = 270;
/** Vertical drag (px) for the whole range; ⇧ makes it ten times finer. */
export const DRAG_PX = 160;
export const FINE = 10;

/** Gain knob law: the fader law (−60…+6 dB) as a 0–1 position. */
export const MIN_DB = -60;
export const MAX_DB = 6;

export function gainToKnob(gain: number): number {
  if (gain <= 0) return 0;
  const db = 20 * Math.log10(gain);
  return Math.min(1, Math.max(0, (db - MIN_DB) / (MAX_DB - MIN_DB)));
}

export function knobToGain(v: number): number {
  if (v <= 0) return 0;
  return 10 ** ((MIN_DB + Math.min(1, v) * (MAX_DB - MIN_DB)) / 20);
}

/** Pan −1…1 as a 0–1 position (centre 0.5). */
export const panToKnob = (pan: number): number => (Math.min(1, Math.max(-1, pan)) + 1) / 2;
export const knobToPan = (v: number): number => Math.min(1, Math.max(-1, v * 2 - 1));

/** The pointer angle (degrees, 0 = straight up, clockwise) of position `v` (0–1). */
export const knobAngle = (v: number): number => -SWEEP_DEGREES / 2 + Math.min(1, Math.max(0, v)) * SWEEP_DEGREES;

/** Position after a vertical drag of `dy` px (negative: up) from `start`. */
export function dragTo(start: number, dy: number, fine: boolean): number {
  return Math.min(1, Math.max(0, start - dy / (DRAG_PX * (fine ? FINE : 1))));
}

/** Which of `count` dots around the sweep are lit for position `v`: from the start for a
 * one-sided knob (gain), from the centre for a bipolar one (pan). */
export function litDots(v: number, count: number, bipolar: boolean): boolean[] {
  return Array.from({ length: count }, (_, i) => {
    const at = i / (count - 1);
    if (!bipolar) return at <= v + 1e-6;
    const lo = Math.min(0.5, v), hi = Math.max(0.5, v);
    return at >= lo - 1e-6 && at <= hi + 1e-6;
  });
}
