<script lang="ts">
  import type { Snippet } from "svelte";
  import { footerSlot } from "./context";

  /**
   * The view's one primary action, centred at the bottom (AGENTS.md §8.6). `children` is the big
   * button; `side` holds the few secondary controls that belong next to it (e.g. Pause, Finish).
   * It renders nothing in place: SessionScreen draws it in the window's footer.
   */
  let { children, side }: { children: Snippet; side?: Snippet } = $props();
  const slot = footerSlot();
  const owner = Symbol("bottom-action");
  $effect(() => {
    slot.show(owner, children, side);
    return () => slot.hide(owner);
  });
</script>
