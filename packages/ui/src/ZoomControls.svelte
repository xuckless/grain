<script lang="ts">
  import Icon from "./Icon.svelte";
  import { fitView, isFit, zoomAt } from "./viewport";
  import type { Session } from "./state";

  /** Zoom out, in and fit-to-song for the shared timeline viewport (AGENTS.md §8.6); ⌘-scroll and
   * pinch on any lane do the same around the pointer. */
  let { session }: { session: Session } = $props();

  const STEP = 1.6;
  const zoom = (factor: number) => (session.viewport = zoomAt(session.viewport, factor, 0.5, session.duration));
</script>

<div class="zoom" role="group" aria-label="Zoom">
  <button class="press icon quiet" aria-label="Zoom out" disabled={isFit(session.viewport, session.duration)} onclick={() => zoom(STEP)}><Icon name="zoomOut" /></button>
  <button class="press icon quiet" aria-label="Zoom in" onclick={() => zoom(1 / STEP)}><Icon name="zoomIn" /></button>
  <button class="press icon quiet" aria-label="Fit the song" title="Fit the song" disabled={isFit(session.viewport, session.duration)} onclick={() => (session.viewport = fitView(session.duration))}><Icon name="fit" /></button>
</div>

<style>
  .zoom {
    display: flex;
    gap: 2px;
  }
</style>
