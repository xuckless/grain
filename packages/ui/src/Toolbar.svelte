<script lang="ts" module>
  import type { Snippet } from "svelte";
  /** One entry of a toolbar's ⋯ overflow menu: rare actions live here (AGENTS.md §8.6). */
  export type OverflowItem = { label: string; run: () => void; disabled?: string | null };
  /** A control that gives way when the toolbar runs short of room: it moves into the ⋯ popover.
   * `key` names it (stable across renders); `control` draws it, inline or in the popover. */
  export type ToolbarItem = { key: string; control: Snippet };
</script>

<script lang="ts">
  import Icon from "./Icon.svelte";
  import { enter, popover } from "./motion";

  /**
   * The session toolbar every view shares (AGENTS.md §8.6): the view's controls left to right by
   * how often they're used, then on the right an (i) popover holding every non-blocking hint and
   * a ⋯ menu for rare actions. Only blocking errors are shown inline by the view itself.
   *
   * `children` always stay in the bar. `items` follow them in priority order: when they don't all
   * fit, the last ones move into the ⋯ popover until the rest do (a ResizeObserver measures; each
   * item's width is remembered from when it was last inline).
   */
  let {
    children,
    items = [],
    right,
    hints = [],
    overflow = [],
  }: {
    children: Snippet;
    items?: readonly ToolbarItem[];
    /** Controls that belong at the right edge, before (i) and ⋯ (e.g. zoom). */
    right?: Snippet;
    hints?: readonly string[];
    overflow?: readonly OverflowItem[];
  } = $props();

  type Open = { kind: "closed" } | { kind: "hints" } | { kind: "overflow" };
  let open = $state<Open>({ kind: "closed" });
  let root = $state<HTMLDivElement | undefined>();

  /** Gap between controls in the bar, px (matches `.main`). */
  const GAP = 12;
  /** How many of `items` fit inline; the rest are in the ⋯ popover. Starts with all inline so
   * every item is measured once. */
  let inline = $state(Number.MAX_SAFE_INTEGER);
  let main = $state<HTMLDivElement | undefined>();
  let fixed = $state<HTMLDivElement | undefined>();
  /** Each item's width when last inline, by key. Not reactive: only `fit` reads it. */
  const widths = new Map<string, number>();
  const shown = $derived(items.slice(0, inline));
  const folded = $derived(items.slice(inline));

  /** The largest prefix of `items` whose remembered widths fit beside the fixed controls. */
  function fit() {
    if (!main || !fixed) return;
    for (const el of main.querySelectorAll<HTMLElement>(":scope > [data-item]")) widths.set(el.dataset.item!, el.offsetWidth);
    let room = main.clientWidth - fixed.offsetWidth;
    let n = 0;
    for (const item of items) {
      room -= GAP + (widths.get(item.key) ?? 0);
      if (room < 0) break;
      n++;
    }
    inline = n;
  }

  // Re-fit when the bar, the fixed controls or any inline item changes size, and when the items
  // themselves change.
  const observer = new ResizeObserver(() => fit());
  const measured = (node: HTMLElement) => {
    observer.observe(node);
    return { destroy: () => observer.unobserve(node) };
  };
  $effect(() => {
    void items;
    fit();
  });
  $effect(() => () => observer.disconnect());

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
  <div class="main" bind:this={main} use:measured>
    <div class="fixed" bind:this={fixed} use:measured>{@render children()}</div>
    {#each shown as item (item.key)}
      <div class="item" data-item={item.key} use:measured>{@render item.control()}</div>
    {/each}
  </div>
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
    {#if overflow.length > 0 || folded.length > 0}
      <div class="anchor">
        <button class="press icon quiet" aria-label="More actions" aria-haspopup="dialog" aria-expanded={open.kind === "overflow"} onclick={() => toggle("overflow")}>
          <Icon name="more" />
        </button>
        {#if open.kind === "overflow"}
          <div class="pop menu" role="dialog" aria-label="More" transition:popover>
            {#if folded.length > 0}
              <div class="folded" role="group" aria-label="More controls">
                {#each folded as item (item.key)}{@render item.control()}{/each}
              </div>
            {/if}
            {#if overflow.length > 0}
              <div class="actions" role="menu">
                {#each overflow as item (item.label)}
                  <button class="action" role="menuitem" disabled={!!item.disabled} title={item.disabled ?? undefined} onclick={() => run(item)}>{item.label}</button>
                {/each}
              </div>
            {/if}
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
  }
  .fixed,
  .item {
    flex: none;
    display: flex;
    align-items: center;
    gap: 12px;
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
    width: max-content;
    min-width: 240px;
    padding: 6px;
    gap: 6px;
  }
  .folded {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 14px;
    padding: 8px 6px;
  }
  .folded + .actions {
    border-top: 1px solid var(--c-surface);
    padding-top: 6px;
  }
  .actions {
    display: flex;
    flex-direction: column;
  }
  .action {
    height: 40px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    text-align: left;
    padding: 0 12px;
    font-size: 15px;
    cursor: pointer;
  }
  .action:hover:not(:disabled) {
    background: var(--c-surface);
  }
  .action:disabled {
    color: var(--c-disabled);
    cursor: default;
  }
</style>
