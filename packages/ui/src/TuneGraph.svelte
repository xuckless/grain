<script lang="ts">
  import type { TunePreview } from "@workstation/engine-api";
  import { pitchRange, targetRuns } from "./autotune";
  import { dot, dotBar, fit } from "./grain";
  import { identity, reveal } from "./motion";
  import { colour, stemHue } from "./theme";
  import { timeToX, viewportGestures, xToTime, type View } from "./viewport";

  /**
   * What auto-tune does to a take, before it is rendered: the pitch as sung (grey grains, the
   * design's Pitch trail), the notes it is pulled to (dotted bars), and the pitch after
   * correction (vocal-hue grains). Drawn over the shared timeline viewport (⌘-scroll or pinch
   * to zoom, AGENTS.md §8.6); click to play from there.
   */
  let {
    preview,
    view,
    duration,
    onView,
    onSeek,
  }: { preview: TunePreview; view: View; duration: number; onView: (v: View) => void; onSeek: (seconds: number) => void } = $props();

  const HEIGHT = 280;

  let canvas = $state<HTMLCanvasElement | undefined>();
  let width = $state(0);

  const voiced = $derived(preview.sung.some((m) => m !== null));

  $effect(() => {
    if (!canvas || width <= 0) return;
    const c = fit(canvas, width, HEIGHT);
    const range = pitchRange(preview);
    if (!c || !range || !voiced) return;
    const { lo, hi } = range;
    const hop = preview.hop_seconds;
    const x = (seconds: number) => timeToX(view, seconds, width);
    const y = (midi: number) => HEIGHT - 8 - ((midi - lo) / (hi - lo)) * (HEIGHT - 16);
    const rows = Math.max(1, Math.floor((HEIGHT - 16) / (hi - lo) / 4));

    // Octave guides (every C).
    for (let m = Math.ceil(lo); m <= hi; m++) if (m % 12 === 0) for (let gx = 0; gx < width; gx += 8) dot(c, gx, y(m), 0.6, colour.disabled);
    for (const run of targetRuns(preview)) dotBar(c, x(run.start * hop), Math.max(x(run.start * hop), x(run.end * hop)), y(run.midi), colour.disabled, 0.8, rows, 4);
    preview.sung.forEach((m, i) => m !== null && dot(c, x(i * hop), y(m), 1.1, colour.muted));
    preview.corrected.forEach((m, i) => m !== null && dot(c, x(i * hop), y(m), 1.3, stemHue.Vocals));
  });

  function click(event: MouseEvent) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    onSeek(Math.max(0, xToTime(view, event.clientX - rect.left, rect.width)));
  }
</script>

<div class="graph" bind:clientWidth={width} use:viewportGestures={{ get: () => view, set: onView, duration }}>
  {#if voiced}
    <button class="canvas-button" onclick={click} aria-label="Pitch before and after auto-tune: click to play from there">
      <canvas bind:this={canvas} use:reveal={identity(preview)} style:height="{HEIGHT}px"></canvas>
    </button>
    <div class="legend">
      <span><i style:background={colour.muted}></i>As sung</span>
      <span><i style:background={colour.disabled}></i>Target note</span>
      <span><i style:background={stemHue.Vocals}></i>Corrected</span>
    </div>
  {:else}
    <p class="muted">This take has no sung notes to tune.</p>
  {/if}
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
  p {
    margin: 0;
  }
</style>
