<script lang="ts">
  import { knobStretch, rubberBand } from "./motion";

  /**
   * A dot-grain slider (design: Components → Gain). While held, the thumb follows the pointer
   * directly (rule 14) and stretches along its travel (rule 5); dragged past either end it gives
   * with damped resistance and springs back on release (rule 19). A hidden range input carries
   * keyboard and screen-reader access. `readout` is the value as shown to the right of the track
   * (and read out to screen readers).
   */
  let {
    value,
    min,
    max,
    step,
    width,
    label,
    readout,
    onChange,
  }: {
    value: number;
    min: number;
    max: number;
    step: number;
    width: number;
    label: string;
    readout: string;
    onChange: (v: number) => void;
  } = $props();

  const THUMB = 20;
  const RUBBER_LIMIT = 44;
  /** Per-frame decay of the overshoot and stretch once released. */
  const SETTLE = 0.8;

  type Drag = { kind: "idle" } | { kind: "dragging"; lastX: number; lastMove: number } | { kind: "settling" };
  let drag = $state<Drag>({ kind: "idle" });
  let offset = $state(0);
  let stretch = $state(1);
  let dir = $state(1);
  let track = $state<HTMLDivElement | undefined>();

  const travel = $derived(width - THUMB);
  const fraction = $derived((value - min) / (max - min));
  const dotCount = $derived(Math.max(6, Math.floor(travel / 10) + 1));
  const dots = $derived(Array.from({ length: dotCount }, (_, i) => i / (dotCount - 1) <= fraction + 1e-6));

  function local(e: PointerEvent): number {
    const r = track!.getBoundingClientRect();
    return (e.clientX - r.left) * (width / r.width) - THUMB / 2;
  }

  function update(e: PointerEvent) {
    if (drag.kind !== "dragging") return;
    const x = local(e);
    const over = x < 0 ? x : x > travel ? x - travel : 0;
    offset = rubberBand(over, RUBBER_LIMIT);
    const vx = x - drag.lastX;
    if (Math.abs(vx) > 0.2) dir = Math.sign(vx);
    stretch = Math.max(stretch, knobStretch(vx));
    drag = { kind: "dragging", lastX: x, lastMove: performance.now() };
    const raw = min + Math.max(0, Math.min(1, x / travel)) * (max - min);
    const snapped = Math.round(raw / step) * step;
    if (snapped !== value) onChange(snapped);
  }

  function down(e: PointerEvent) {
    track!.setPointerCapture(e.pointerId);
    drag = { kind: "dragging", lastX: local(e), lastMove: performance.now() };
    update(e);
  }

  function up() {
    if (drag.kind === "dragging") drag = { kind: "settling" };
  }

  // Relax the stretch while held still, and settle overshoot + stretch after release.
  $effect(() => {
    if (drag.kind === "idle") return;
    let raf = requestAnimationFrame(function tick() {
      const now = performance.now();
      if (drag.kind === "settling") offset *= SETTLE;
      if (drag.kind === "settling" || (drag.kind === "dragging" && now - drag.lastMove > 40)) stretch = 1 + (stretch - 1) * SETTLE;
      if (drag.kind === "settling" && Math.abs(offset) < 0.1 && stretch - 1 < 0.002) {
        offset = 0;
        stretch = 1;
        drag = { kind: "idle" };
        return;
      }
      raf = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(raf);
  });
</script>

<div class="slider">
  <div
    class="track"
    style:width="{width}px"
    bind:this={track}
    onpointerdown={down}
    onpointermove={update}
    onpointerup={up}
    onpointercancel={up}
    role="presentation"
  >
    <div class="dots" aria-hidden="true">
      {#each dots as on, i (i)}<span class:on></span>{/each}
    </div>
    <div
      class="thumb"
      aria-hidden="true"
      style:transform="translateX({fraction * travel + offset}px) scaleX({stretch}) scaleY({1 - (stretch - 1) * 0.4})"
      style:transform-origin={dir > 0 ? "30% 50%" : "70% 50%"}
    ></div>
    <input
      type="range"
      {min}
      {max}
      {step}
      {value}
      aria-label={label}
      aria-valuetext={readout}
      oninput={(e) => onChange(Number(e.currentTarget.value))}
    />
  </div>
  <span class="value mono" aria-hidden="true">{readout}</span>
</div>

<style>
  .slider {
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .value {
    min-width: 6ch;
    font-size: 12px;
    line-height: 16px;
    color: var(--c-muted);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .track {
    flex: none;
    position: relative;
    height: 44px;
    cursor: pointer;
    touch-action: none;
  }
  .dots {
    position: absolute;
    left: 10px;
    right: 10px;
    top: 20px;
    height: 4px;
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .dots span {
    width: 2px;
    height: 2px;
    border-radius: 50%;
    background: var(--c-muted);
    flex: none;
  }
  .dots span.on {
    width: 4px;
    height: 4px;
    background: var(--c-text);
  }
  .thumb {
    position: absolute;
    left: 0;
    top: 12px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--c-text);
    will-change: transform;
  }
  input {
    position: absolute;
    inset: 0;
    opacity: 0;
    pointer-events: none;
    margin: 0;
  }
  .track:has(input:focus-visible) {
    outline: 2px solid var(--c-text);
    outline-offset: 2px;
    border-radius: 12px;
  }
</style>
