<script lang="ts">
  import { grain, type Behaviour } from "./grain";

  /** A canvas running one grain behaviour; size it with `style`. Decorative unless `label` is set. */
  let { behaviour, style = "", label }: { behaviour: Behaviour; style?: string; label?: string } = $props();
</script>

{#if label}
  <div {style} role="img" aria-label={label}><canvas use:grain={behaviour} aria-hidden="true"></canvas></div>
{:else}
  <div {style} aria-hidden="true"><canvas use:grain={behaviour}></canvas></div>
{/if}

<style>
  /* The canvas never sizes its box (its backing store follows the box, so letting it size
     the box would feed back); the wrapper takes its size from `style`. */
  div {
    position: relative;
  }
  canvas {
    position: absolute;
    inset: 0;
    display: block;
    width: 100%;
    height: 100%;
  }
</style>
