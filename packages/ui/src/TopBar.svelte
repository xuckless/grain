<script lang="ts">
  import type { Snippet } from "svelte";
  import type { WindowChrome } from "@workstation/engine-api";

  /**
   * The window's top bar. It blends into the window frame: same ground colour, no border, and
   * on macOS it leaves room for the window buttons (which sit over it, vertically centred) and is
   * the drag area. Only non-interactive parts carry `data-tauri-drag-region`; double-clicking
   * them zooms the window like a native title bar.
   */
  let { chrome, left, center, right }: { chrome: WindowChrome; left?: Snippet; center?: Snippet; right?: Snippet } = $props();
</script>

<header data-tauri-drag-region style:padding-left="{chrome.kind === 'overlay' ? chrome.insetLeft : 16}px">
  <div class="side" data-tauri-drag-region>{@render left?.()}</div>
  <div class="center" data-tauri-drag-region>{@render center?.()}</div>
  <div class="side end" data-tauri-drag-region>{@render right?.()}</div>
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
  .end {
    justify-content: flex-end;
  }
</style>
