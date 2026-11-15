<script lang="ts">
  import type { Snippet } from "svelte";
  import type { WindowChrome } from "@workstation/engine-api";

  /**
   * The window's top bar. It blends into the window frame: same ground colour, no border, and
   * on macOS it leaves room for the window buttons (which sit over it, vertically centred) and is
   * the drag area: pressing anywhere on it except a control drags the window, and double-clicking
   * zooms it like a native title bar. (`data-tauri-drag-region` only reacts on the exact element
   * that carries it, so text and wrappers inside the bar would block dragging.)
   */
  let { chrome, left, center, right }: { chrome: WindowChrome; left?: Snippet; center?: Snippet; right?: Snippet } = $props();

  /** Elements that keep their own pointer behaviour inside the bar. */
  const INTERACTIVE = "button, a, input, select, textarea, label, [role='button'], [role='slider'], [role='switch'], [role='tab'], [role='menu'], [role='listbox'], [role='option'], [role='dialog']";

  function onmousedown(e: MouseEvent) {
    if (chrome.kind !== "overlay" || e.button !== 0) return;
    if (e.target instanceof Element && e.target.closest(INTERACTIVE)) return;
    e.preventDefault();
    if (e.detail === 2) chrome.toggleMaximize();
    else chrome.startDrag();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions (window dragging is a pointer-only affordance; every control inside stays a real button) -->
<header {onmousedown} class:draggable={chrome.kind === "overlay"} style:padding-left="{chrome.kind === 'overlay' ? chrome.insetLeft : 16}px">
  <div class="side">{@render left?.()}</div>
  <div class="center">{@render center?.()}</div>
  <div class="side end">{@render right?.()}</div>
</header>

<style>
  header {
    height: 64px;
    flex: none;
    box-sizing: border-box;
    padding-right: 16px;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
    align-items: center;
    gap: 16px;
    background: var(--c-ground);
    user-select: none;
    -webkit-user-select: none;
  }
  .side,
  .center {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    height: 100%;
  }
  .draggable {
    cursor: default;
  }
  .end {
    justify-content: flex-end;
  }
</style>
