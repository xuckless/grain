<script lang="ts">
  import Grain from "./Grain.svelte";
  import { onFrame } from "./clock";
  import { services } from "./context";
  import { DotProgress, Gather } from "./grain";
  import { popover } from "./motion";

  /**
   * A short job in progress, in Home's import style: grains gather into a ring over a dot
   * progress bar. Jobs that report no fraction (saving a take, calibrating) ease towards full
   * over about `expected` seconds; `done` fills it and confirms with a hit burst. It floats over
   * its positioned parent's centre.
   */
  let { label, expected, done = false }: { label: string; expected: number; done?: boolean } = $props();

  const { burst } = services();
  const gather = new Gather();
  const bar = new DotProgress();
  let card = $state<HTMLElement | undefined>();

  /** Never quite full until the job says it's done. */
  const EASED_MAX = 0.92;

  $effect(() => {
    if (done) {
      gather.progress = bar.progress = 1;
      if (card) burst.fireAt(card);
      return;
    }
    const t0 = performance.now();
    return onFrame((now) => {
      gather.progress = bar.progress = EASED_MAX * (1 - Math.exp(-(now - t0) / 1000 / expected));
    });
  });
</script>

<div class="working" bind:this={card} role="status" aria-live="polite" transition:popover>
  <Grain behaviour={gather} style="width: 72px; height: 72px; flex: none" />
  <div class="text">
    <span>{label}</span>
    <Grain behaviour={bar} style="width: 180px; height: 20px" />
  </div>
</div>

<style>
  .working {
    position: absolute;
    left: 50%;
    top: 50%;
    translate: -50% -50%;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px 22px 12px 12px;
    border-radius: 16px;
    background: var(--c-raised);
    box-shadow: 0 12px 40px color-mix(in srgb, var(--c-ground) 70%, transparent);
    z-index: 10;
    pointer-events: none;
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: 15px;
    font-weight: 500;
  }
</style>
