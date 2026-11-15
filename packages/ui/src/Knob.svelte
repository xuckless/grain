<script lang="ts">
  import { dragTo, knobAngle, litDots } from "./knob";
  import { popover } from "./motion";

  /**
   * A dot-grain rotary knob (AGENTS.md §8.6, Mix): a ring of dots lit along its value, a pointer
   * dot, the value shown while turning or hovering. Drag up/down to turn (⇧ for fine), arrow keys
   * step, double-click resets. While held it keeps its own position and sends at most one change
   * per frame (§10.1); `onCommit` fires once when a turn ends, with the value it started from.
   */
  let {
    value,
    reset,
    label,
    format,
    bipolar = false,
    colour = "var(--c-text)",
    onChange,
    onCommit,
  }: {
    /** 0–1. */
    value: number;
    /** Where double-click returns to (0–1). */
    reset: number;
    label: string;
    format: (v: number) => string;
    bipolar?: boolean;
    colour?: string;
    onChange: (v: number) => void;
    onCommit: (from: number, to: number) => void;
  } = $props();

  const SIZE = 36;
  const DOTS = 15;
  const R = 15;

  type Turn = { kind: "idle" } | { kind: "turning"; startY: number; startValue: number; local: number; pointer: number };
  let turn = $state<Turn>({ kind: "idle" });
  let hover = $state(false);
  /** A frame is queued to send the newest position. */
  let queued = 0;

  const shown = $derived(turn.kind === "turning" ? turn.local : value);
  const lit = $derived(litDots(shown, DOTS, bipolar));
  const ring = Array.from({ length: DOTS }, (_, i) => {
    const a = (knobAngle(i / (DOTS - 1)) * Math.PI) / 180;
    return { x: SIZE / 2 + R * Math.sin(a), y: SIZE / 2 - R * Math.cos(a) };
  });
  const pointer = $derived.by(() => {
    const a = (knobAngle(shown) * Math.PI) / 180;
    return { x: SIZE / 2 + 8 * Math.sin(a), y: SIZE / 2 - 8 * Math.cos(a) };
  });

  function send() {
    if (queued) return;
    queued = requestAnimationFrame(() => {
      queued = 0;
      if (turn.kind === "turning") onChange(turn.local);
    });
  }

  function down(e: PointerEvent) {
    if (e.button !== 0) return;
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    turn = { kind: "turning", startY: e.clientY, startValue: value, local: value, pointer: e.pointerId };
  }

  function move(e: PointerEvent) {
    if (turn.kind !== "turning" || e.pointerId !== turn.pointer) return;
    turn = { ...turn, local: dragTo(turn.startValue, e.clientY - turn.startY, e.shiftKey) };
    send();
  }

  function up(e: PointerEvent) {
    if (turn.kind !== "turning" || e.pointerId !== turn.pointer) return;
    const { startValue, local } = turn;
    cancelAnimationFrame(queued);
    queued = 0;
    turn = { kind: "idle" };
    onChange(local);
    onCommit(startValue, local);
  }

  function set(v: number) {
    const to = Math.min(1, Math.max(0, v));
    onChange(to);
    onCommit(value, to);
  }

  function key(e: KeyboardEvent) {
    const step = e.shiftKey ? 0.005 : 0.025;
    if (e.key === "ArrowUp" || e.key === "ArrowRight") set(value + step);
    else if (e.key === "ArrowDown" || e.key === "ArrowLeft") set(value - step);
    else return;
    e.preventDefault();
  }

  $effect(() => () => cancelAnimationFrame(queued));
</script>

<div class="knob" onpointerenter={() => (hover = true)} onpointerleave={() => (hover = false)} role="presentation">
  <svg
    width={SIZE}
    height={SIZE}
    viewBox="0 0 {SIZE} {SIZE}"
    role="slider"
    tabindex="0"
    aria-label={label}
    aria-valuemin={0}
    aria-valuemax={1}
    aria-valuenow={shown}
    aria-valuetext={format(shown)}
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
    onpointercancel={up}
    ondblclick={() => set(reset)}
    onkeydown={key}
  >
    {#each ring as d, i (i)}
      <circle cx={d.x} cy={d.y} r={lit[i] ? 1.7 : 1.2} fill={lit[i] ? colour : "var(--c-disabled)"}></circle>
    {/each}
    <circle cx={SIZE / 2} cy={SIZE / 2} r="10" fill="var(--c-raised)"></circle>
    <circle cx={pointer.x} cy={pointer.y} r="2" fill="var(--c-text)"></circle>
  </svg>
  {#if turn.kind === "turning" || hover}
    <span class="value mono" transition:popover>{format(shown)}</span>
  {/if}
</div>

<style>
  .knob {
    position: relative;
    width: 44px;
    height: 44px;
    display: grid;
    place-items: center;
    flex: none;
  }
  svg {
    cursor: ns-resize;
    touch-action: none;
    outline: none;
    border-radius: 50%;
  }
  svg:focus-visible {
    box-shadow: 0 0 0 2px var(--c-muted);
  }
  .value {
    position: absolute;
    bottom: 40px;
    left: 50%;
    translate: -50% 0;
    padding: 2px 8px;
    border-radius: 6px;
    background: var(--c-raised);
    font-size: 12px;
    line-height: 18px;
    white-space: nowrap;
    pointer-events: none;
    transform-origin: bottom center;
    z-index: 10;
  }
</style>
