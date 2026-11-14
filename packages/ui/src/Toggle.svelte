<script lang="ts">
  import { dur, KNOB_MAX_STRETCH, reducedMotion } from "./motion";

  /** A switch (design: Components → Toggle): the fill wipes in with a mask, the knob stretches as it moves. */
  let { checked, label, onToggle }: { checked: boolean; label: string; onToggle: (next: boolean) => void } = $props();

  let knob = $state<HTMLSpanElement | undefined>();

  function toggle() {
    onToggle(!checked);
    if (knob && !reducedMotion()) {
      const s = 1 + KNOB_MAX_STRETCH;
      knob.animate([{ transform: "scaleX(1)" }, { transform: `scaleX(${s})`, offset: 0.45 }, { transform: "scaleX(1)" }], { duration: dur.toggle });
    }
  }
</script>

<div class="row">
  <button class="press switch" class:on={checked} role="switch" aria-checked={checked} aria-label={label} onclick={toggle}>
    <span class="fill" aria-hidden="true"></span>
    <span class="knob" aria-hidden="true"><span class="inner" bind:this={knob}></span></span>
  </button>
  <span>{label}</span>
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 14px;
  }
  .switch {
    position: relative;
    width: 52px;
    height: 30px;
    border-radius: 8px;
    background: var(--c-raised);
    padding: 0;
    flex: none;
  }
  /* A 30 px switch inside a 44 px touch target. */
  .switch::after {
    content: "";
    position: absolute;
    inset: -7px -4px;
  }
  .fill {
    position: absolute;
    inset: 0;
    background: var(--c-text);
    border-radius: 8px;
    clip-path: inset(0 100% 0 0 round 8px);
    transition: clip-path var(--dur-toggle) var(--ease-io);
  }
  .on .fill {
    clip-path: inset(0 0 0 0 round 8px);
  }
  .knob {
    position: absolute;
    left: 4px;
    top: 4px;
    width: 22px;
    height: 22px;
    display: block;
    transition: transform var(--dur-toggle) var(--ease-io);
  }
  .on .knob {
    transform: translateX(22px);
  }
  .inner {
    display: block;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    background: var(--c-muted);
    transition: background-color var(--dur-toggle) var(--ease-io);
  }
  .on .inner {
    background: var(--c-ground);
  }
</style>
