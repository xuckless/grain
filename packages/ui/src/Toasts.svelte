<script lang="ts" module>
  import type { ToastTone } from "./context";
  export type Toast = { id: number; text: string; tone: ToastTone };
</script>

<script lang="ts">
  import Grain from "./Grain.svelte";
  import { Dissolve } from "./grain";
  import { dur, enter } from "./motion";

  /**
   * One toast at a time, bottom centre. It stays for `dur.toast`, then its text turns into
   * grains that drift away (design: Particles → Dissolve). A newer toast replaces it.
   */
  let { toast }: { toast: Toast | null } = $props();

  type ToastState =
    | { kind: "none" }
    | { kind: "shown"; toast: Toast }
    | { kind: "dissolving"; toast: Toast; grains: Dissolve; width: number };
  let phase = $state<ToastState>({ kind: "none" });
  let label = $state<HTMLSpanElement | undefined>();

  const FONT = '400 15px "Geist Variable", Geist, sans-serif';

  $effect(() => {
    if (!toast) return;
    const t = toast;
    phase = { kind: "shown", toast: t };
    const leave = setTimeout(() => {
      if (phase.kind !== "shown" || phase.toast.id !== t.id) return;
      const grains = new Dissolve(t.text, FONT);
      phase = { kind: "dissolving", toast: t, grains, width: label?.offsetWidth ?? 200 };
      requestAnimationFrame(() => grains.start());
    }, dur.toast);
    const gone = setTimeout(() => {
      if (phase.kind !== "none" && phase.toast.id === t.id) phase = { kind: "none" };
    }, dur.toast + dur.dissolve);
    return () => {
      clearTimeout(leave);
      clearTimeout(gone);
    };
  });
</script>

<div class="dock" aria-live="polite">
  {#if phase.kind === "shown"}
    {#key phase.toast.id}
      <div class="toast" class:error={phase.toast.tone === "error"} in:enter>
        {#if phase.toast.tone === "error"}<span class="mark" aria-hidden="true"></span>{/if}
        <span bind:this={label}>{phase.toast.text}</span>
      </div>
    {/key}
  {:else if phase.kind === "dissolving"}
    <div class="ghost" aria-hidden="true">
      <Grain behaviour={phase.grains} style="width: {phase.width + 120}px; height: 140px" />
    </div>
  {/if}
</div>

<style>
  .dock {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 32px;
    display: flex;
    justify-content: center;
    pointer-events: none;
    z-index: 50;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 20px;
    background: var(--c-raised);
    border-radius: 12px;
    font-size: 15px;
    line-height: 22px;
  }
  .mark {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--c-near);
  }
  .ghost {
    margin-bottom: -48px;
  }
</style>
