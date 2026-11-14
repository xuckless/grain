// Grain colours: the single source for both CSS (via custom properties) and canvas drawing.
// Colour only ever means sound: each stem keeps its hue everywhere; the interface itself stays
// white on black. Feedback colours also differ in lightness so they read without colour vision.

import type { Stem, TrackSource } from "@workstation/engine-api";

export const colour = {
  ground: "#050506",
  surface: "#0E0E11",
  raised: "#17171B",
  text: "#F2F2F0",
  muted: "#9A9AA2",
  disabled: "#5E5E66",
} as const;

export const stemHue: Record<Stem, string> = {
  Vocals: "#F0449A",
  Drums: "#FF6B3D",
  Bass: "#4F7DFF",
  Guitar: "#3FD9D0",
  Piano: "#B6E34B",
  Other: "#E7D6B4",
};

/** Hit / near / miss feedback (karaoke lane, score graph). */
export const feedback = {
  hit: "#FFFFFF",
  near: "#FFB547",
  miss: "#8E6BFF",
} as const;

/** The stem hues in display order, for multi-colour grain (bursts, terrain, progress). */
export const spectrum: readonly string[] = [stemHue.Vocals, stemHue.Drums, stemHue.Bass, stemHue.Guitar, stemHue.Piano];

/** A track's hue: its stem's, or plain text for an unseparated mix. */
export function trackHue(source: TrackSource): string {
  switch (source.kind) {
    case "Mix":
      return colour.text;
    case "Stem":
      return stemHue[source.stem];
  }
}

/** Publishes the palette as CSS custom properties (`--c-ground`, `--c-vocals`, …) on :root. */
export function installTheme(): void {
  const s = document.documentElement.style;
  for (const [name, value] of Object.entries(colour)) s.setProperty(`--c-${name}`, value);
  for (const [name, value] of Object.entries(stemHue)) s.setProperty(`--c-${name.toLowerCase()}`, value);
  for (const [name, value] of Object.entries(feedback)) s.setProperty(`--c-${name}`, value);
}
