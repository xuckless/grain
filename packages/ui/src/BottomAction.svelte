<script lang="ts">
  import type { Snippet } from "svelte";
  import { enter } from "./motion";

  /**
   * The view's one primary action, centred at the bottom (AGENTS.md §8.6). `children` is the big
   * button; `side` holds the few secondary controls that belong next to it (e.g. Pause, Finish).
   */
  let { children, side }: { children: Snippet; side?: Snippet } = $props();
</script>

<div class="bottom" in:enter={{ delay: 60 }}>
  <div class="side"></div>
  <div class="main">{@render children()}</div>
  <div class="side end">{@render side?.()}</div>
</div>

<style>
  .bottom {
    position: sticky;
    bottom: 0;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: 16px;
    padding: 16px 0 4px;
    background: linear-gradient(to top, var(--c-ground) 70%, transparent);
    z-index: 5;
  }
  .side {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .end {
    justify-content: flex-end;
  }
  .main :global(.cta) {
    height: 56px;
    min-width: 200px;
    padding: 0 28px;
    border: 0;
    border-radius: 28px;
    background: var(--c-text);
    color: var(--c-ground);
    font-size: 17px;
    font-weight: 500;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
  }
  .main :global(.cta:disabled) {
    background: var(--c-surface);
    color: var(--c-disabled);
  }
</style>
