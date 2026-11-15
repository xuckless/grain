<script lang="ts" generics="T extends string">
  import Icon from "./Icon.svelte";
  import { dur, enter, popover } from "./motion";

  /** A dropdown listbox (design: Components → Menu). Opens from its trigger (rules 10–12). */
  let {
    label,
    value,
    options,
    onPick,
  }: { label: string; value: T; options: readonly { id: T; label: string }[]; onPick: (id: T) => void } = $props();

  type MenuState = { kind: "closed" } | { kind: "open" };
  let menu = $state<MenuState>({ kind: "closed" });
  let root = $state<HTMLDivElement | undefined>();

  const current = $derived(options.find((o) => o.id === value)?.label ?? value);

  function pick(id: T) {
    menu = { kind: "closed" };
    onPick(id);
  }

  // Close on a press outside the menu while it is open.
  $effect(() => {
    if (menu.kind !== "open") return;
    const away = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node)) menu = { kind: "closed" };
    };
    window.addEventListener("pointerdown", away);
    return () => window.removeEventListener("pointerdown", away);
  });
</script>

<div class="menu" bind:this={root} onkeydown={(e) => e.key === "Escape" && (menu = { kind: "closed" })} role="presentation">
  <button
    class="press trigger"
    aria-haspopup="listbox"
    aria-expanded={menu.kind === "open"}
    onclick={() => (menu = menu.kind === "open" ? { kind: "closed" } : { kind: "open" })}
  >
    <span class="muted">{label}</span>
    {#key current}<span class="value swap">{current}</span>{/key}
    <span class="chev" class:open={menu.kind === "open"}><Icon name="chevron" size={14} /></span>
  </button>
  {#if menu.kind === "open"}
    <div class="list" role="listbox" aria-label={label} transition:popover>
      {#each options as o, i (o.id)}
        <button class="item" role="option" aria-selected={o.id === value} onclick={() => pick(o.id)} in:enter={{ delay: i * dur.stagger, detail: true }}>
          <span>{o.label}</span>
          {#if o.id === value}<Icon name="check" size={14} />{/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .menu {
    position: relative;
  }
  .trigger {
    height: 44px;
    padding: 0 14px 0 16px;
    border-radius: 12px;
    background: var(--c-surface);
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .trigger:hover {
    background: var(--c-raised);
  }
  .value {
    min-width: 64px;
    text-align: left;
  }
  .chev {
    display: flex;
    transition: transform var(--dur-menu) var(--ease-out);
  }
  .chev.open {
    transform: rotate(180deg);
  }
  .list {
    position: absolute;
    left: 0;
    top: 52px;
    width: 220px;
    box-sizing: border-box;
    padding: 6px;
    background: var(--c-raised);
    border-radius: 12px;
    transform-origin: top left; /* rules 11–12: grows from and shrinks to the trigger */
    z-index: 20;
    display: flex;
    flex-direction: column;
  }
  .item {
    height: 40px;
    border-radius: 8px;
    padding: 0 12px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    transition: background-color var(--dur-press) var(--ease-out);
  }
  .item:hover {
    background: var(--c-surface);
  }
</style>
