<script lang="ts">
  import type { FrameComparison, NoteResult, Project, ScoreReport } from "@workstation/engine-api";
  import { dot, dotBar, fit } from "./grain";
  import { reveal } from "./motion";
  import { colour, feedback } from "./theme";

  /**
   * Where the take was sharp, flat, early or late: reference notes as dotted bars coloured by how
   * they were sung, the singer's pitch as grains, and a cents strip below. Click to play from a note.
   */
  let { project, report, onSeek }: { project: Project; report: ScoreReport; onSeek: (seconds: number) => void } = $props();

  const ROLL = 280;
  const STRIP = 72;
  /** Cents shown at the top/bottom of the error strip. */
  const STRIP_RANGE = 100;
  /** A missed frame within this many cents still reads as "near". */
  const NEAR_CENTS = 100;
  /** Fraction of a note sung in tune for it to count as a hit. */
  const HIT_RATIO = 0.6;

  let canvas = $state<HTMLCanvasElement | undefined>();
  let width = $state(0);
  const height = ROLL + STRIP;
  const duration = $derived(project.frames / project.sample_rate);

  function noteColour(result: NoteResult): string {
    switch (result.kind) {
      case "Missed":
        return feedback.miss;
      case "Sung":
        return result.hit_ratio >= HIT_RATIO ? feedback.hit : feedback.near;
    }
  }

  function frameColour(f: FrameComparison): string | undefined {
    switch (f.kind) {
      case "Hit":
        return feedback.hit;
      case "Miss":
        return Math.abs(f.cents) <= NEAR_CENTS ? feedback.near : feedback.miss;
      case "ReferenceOnly":
      case "UserOnly":
      case "Silent":
        return undefined;
    }
  }

  $effect(() => {
    if (!canvas || width <= 0) return;
    const c = fit(canvas, width, height);
    if (!c) return;
    const notes = report.notes.map((n) => n.note);
    if (notes.length === 0) return;
    const lo = Math.min(...notes.map((n) => n.midi)) - 3;
    const hi = Math.max(...notes.map((n) => n.midi)) + 3;
    const x = (t: number) => (t / duration) * width;
    const y = (midi: number) => ROLL - 8 - ((midi - lo) / (hi - lo)) * (ROLL - 16);
    const rows = Math.max(1, Math.floor((ROLL - 16) / (hi - lo) / 4));

    // Octave guides (every C) as faint dotted lines.
    for (let m = Math.ceil(lo); m <= hi; m++) if (m % 12 === 0) for (let gx = 0; gx < width; gx += 8) dot(c, gx, y(m), 0.6, colour.disabled);

    for (const n of report.notes) dotBar(c, x(n.note.start), Math.max(x(n.note.start), x(n.note.end)), y(n.note.midi), noteColour(n.result), 0.8, rows, 4);

    report.user_pitch.frames.forEach((f, i) => {
      if (f.kind !== "Voiced") return;
      dot(c, x(i * report.hop_seconds), y(f.midi), 1.3, frameColour(report.frames[i]) ?? colour.muted);
    });

    const mid = ROLL + STRIP / 2;
    for (let gx = 0; gx < width; gx += 8) dot(c, gx, mid, 0.6, colour.disabled);
    const half = STRIP / 2 - 6;
    report.frames.forEach((f, i) => {
      if (f.kind !== "Hit" && f.kind !== "Miss") return;
      const h = (Math.max(-STRIP_RANGE, Math.min(STRIP_RANGE, f.cents)) / STRIP_RANGE) * half;
      const fx = x(i * report.hop_seconds);
      const col = frameColour(f) ?? colour.muted;
      for (let k = 0; k <= Math.abs(h); k += 3) dot(c, fx, mid - Math.sign(h) * k, 0.8, col);
    });
  });

  function click(event: MouseEvent) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const t = ((event.clientX - rect.left) / rect.width) * duration;
    const note = report.notes.find((n) => n.note.start <= t && t <= n.note.end);
    onSeek(note ? note.note.start : t);
  }
</script>

<div class="graph" bind:clientWidth={width}>
  <button class="canvas-button" onclick={click} aria-label="Pitch graph: click a note to play from it">
    <canvas bind:this={canvas} use:reveal={report} style:height="{height}px"></canvas>
  </button>
  <div class="legend">
    <span><i style:background={feedback.hit}></i>Hit</span>
    <span><i style:background={feedback.near}></i>Near</span>
    <span><i style:background={feedback.miss}></i>Miss</span>
    <span class="muted">Strip: cents sharp (up) or flat (down), ±{STRIP_RANGE}¢. Click a note to play from it.</span>
  </div>
</div>

<style>
  .graph {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .canvas-button {
    padding: 0;
    background: none;
    display: block;
    width: 100%;
    cursor: pointer;
  }
  canvas {
    display: block;
    width: 100%;
  }
  .legend {
    display: flex;
    gap: 20px;
    flex-wrap: wrap;
    font-size: 13px;
    line-height: 18px;
  }
  .legend span {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .legend i {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
</style>
