<script lang="ts">
  import { popover } from "./motion";
  import { VIEWS, type ViewDef, type ViewId } from "./state";

  /**
   * Session view tabs (design: Components → Views). The indicator slides with ease-in-out
   * (rule 9). Views that aren't built yet stay visible but disabled; hovering, focusing or
   * pressing one shows why in a popover that grows from the tab (rules 10–12).
   */
  let { view, onSelect }: { view: ViewId; onSelect: (id: ViewId) => void } = $props();

  const TAB_W = 104;
  const GAP = 4;

  type Tip = { kind: "hidden" } | { kind: "shown"; id: ViewId };
  let tip = $state<Tip>({ kind: "hidden" });

  const index = $derived(VIEWS.findIndex((v) => v.id === view));

  function show(v: ViewDef) {
    if (v.availability.kind === "unavailable") tip = { kind: "shown", id: v.id };
  }
  function hide(v: ViewDef) {
    if (tip.kind === "shown" && tip.id === v.id) tip = { kind: "hidden" };
  }
</script>

<div class="tabs" role="tablist" aria-label="Session view">
  <span class="indicator" aria-hidden="true" style:transform="translateX({index * (TAB_W + GAP)}px)"></span>
  {#each VIEWS as v (v.id)}
    {@const ready = v.availability.kind === "ready"}
    <div class="slot">
      <button
        class="press tab"
        class:selected={v.id === view}
        class:unavailable={!ready}
        role="tab"
        aria-selected={v.id === view}
        aria-disabled={!ready}
        aria-describedby={ready ? undefined : `tip-${v.id}`}
        style:width="{TAB_W}px"
        onclick={() => (ready ? onSelect(v.id) : show(v))}
        onpointerenter={() => show(v)}
        onpointerleave={() => hide(v)}
        onfocus={() => show(v)}
        onblur={() => hide(v)}
      >
        {v.label}
      </button>
      {#if v.availability.kind === "unavailable" && tip.kind === "shown" && tip.id === v.id}
        <div class="tip" id="tip-{v.id}" role="tooltip" transition:popover>
          <strong>Unavailable</strong>
          <span>{v.availability.reason}</span>
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
  .tabs {
    position: relative;
    display: flex;
    gap: 4px;
    padding: 4px;
    background: var(--c-surface);
    border-radius: 12px;
    align-self: flex-start;
  }
  .indicator {
    position: absolute;
    left: 4px;
    top: 4px;
    width: 104px;
    height: 40px;
    border-radius: 9px;
    background: var(--c-raised);
    transition: transform var(--dur-tab) var(--ease-io);
  }
  .slot {
    position: relative;
  }
  .tab {
    position: relative;
    height: 40px;
    border-radius: 9px;
    background: transparent;
    color: var(--c-muted);
    transition:
      color var(--dur-colour) var(--ease-out),
      transform var(--dur-press) var(--ease-out);
  }
  .tab.selected {
    color: var(--c-text);
  }
  .tab.unavailable {
    color: var(--c-disabled);
    cursor: default;
  }
  .tab.unavailable:active {
    transform: none;
  }
  .tip {
    position: absolute;
    top: 52px;
    left: 50%;
    width: 240px;
    margin-left: -120px;
    box-sizing: border-box;
    padding: 14px 16px;
    background: var(--c-raised);
    border-radius: 12px;
    font-size: 14px;
    line-height: 21px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    transform-origin: top center; /* grows from the tab above it */
    z-index: 30;
  }
  .tip strong {
    font-weight: 500;
  }
  .tip span {
    color: var(--c-muted);
  }
</style>
