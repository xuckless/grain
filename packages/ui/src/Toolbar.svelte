<script lang="ts" module>
  /** One entry of a toolbar's ⋯ overflow menu: rare actions live here (AGENTS.md §8.6). */
  export type OverflowItem = { label: string; run: () => void; disabled?: string | null };
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import { enter, popover } from "./motion";

  /**
   * The session toolbar every view shares (AGENTS.md §8.6): the view's controls left to right by
   * how often they're used, then on the right an (i) popover holding every non-blocking hint and
   * a ⋯ menu for rare actions. Only blocking errors are shown inline by the view itself.
   */
  let {
    children,
    right,
    hints = [],
    overflow = [],
  }: {
    children: Snippet;
    /** Controls that belong at the right edge, before (i) and ⋯ (e.g. zoom). */
    right?: Snippet;
    hints?: readonly string[];
    overflow?: readonly OverflowItem[];
  } = $props();

  type Open = { kind: "closed" } | { kind: "hints" } | { kind: "overflow" };
  let open = $state<Open>({ kind: "closed" });
  let root = $state<HTMLDivElement | undefined>();

  const toggle = (kind: "hints" | "overflow") => (open = open.kind === kind ? { kind: "closed" } : { kind });

  $effect(() => {
    if (open.kind === "closed") return;
    const away = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node)) open = { kind: "closed" };
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && (open = { kind: "closed" });
    window.addEventListener("pointerdown", away);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("pointerdown", away);
      window.removeEventListener("keydown", esc);
    };
  });

  function run(item: OverflowItem) {
    open = { kind: "closed" };
    if (!item.disabled) item.run();
  }
</script>

<div class="toolbar" role="toolbar" in:enter>
  <div class="main">{@render children()}</div>
  <div class="end" bind:this={root}>
    {@render right?.()}
    {#if hints.length > 0}
      <div class="anchor">
        <button class="press icon quiet" aria-label="Tips for this view" aria-expanded={open.kind === "hints"} onclick={() => toggle("hints")}>
          <Icon name="info" />
          <span class="dot" aria-hidden="true"></span>
        </button>
        {#if open.kind === "hints"}
          <div class="pop" role="dialog" aria-label="Tips" transition:popover>
            {#each hints as hint, i (i)}<p>{hint}</p>{/each}
          </div>
        {/if}
      </div>
    {/if}
    {#if overflow.length > 0}
      <div class="anchor">
        <button class="press icon quiet" aria-label="More actions" aria-haspopup="menu" aria-expanded={open.kind === "overflow"} onclick={() => toggle("overflow")}>
          <Icon name="more" />
        </button>
        {#if open.kind === "overflow"}
          <div class="pop menu" role="menu" transition:popover>
            {#each overflow as item (item.label)}
              <button class="item" role="menuitem" disabled={!!item.disabled} title={item.disabled ?? undefined} onclick={() => run(item)}>{item.label}</button>
            {/each}
          </div>
        {/if}
      </div>
    {/if}
  </div>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 16px;
    min-height: 56px;
    padding: 6px 8px 6px 12px;
    box-sizing: border-box;
    background: var(--c-surface);
    border-radius: 12px;
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .end {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: none;
  }
  .anchor {
    position: relative;
  }
  .anchor > button {
    position: relative;
  }
  .dot {
    position: absolute;
    top: 9px;
    right: 9px;
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--c-near);
  }
  .pop {
    position: absolute;
    right: 0;
    top: 52px;
    width: 320px;
    box-sizing: border-box;
    padding: 14px 16px;
    background: var(--c-raised);
    border-radius: 12px;
    font-size: 14px;
    line-height: 21px;
    transform-origin: top right; /* rules 11–12 */
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pop p {
    margin: 0;
  }
  .menu {
    width: 240px;
    padding: 6px;
    gap: 0;
  }
  .item {
    height: 40px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    text-align: left;
    padding: 0 12px;
    font-size: 15px;
    cursor: pointer;
  }
  .item:hover:not(:disabled) {
    background: var(--c-surface);
  }
  .item:disabled {
    color: var(--c-disabled);
    cursor: default;
  }
</style>
