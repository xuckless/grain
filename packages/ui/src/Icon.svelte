<script lang="ts" module>
  /** Every icon in the UI: inline stroke SVGs on a 20×20 grid, coloured by `currentColor`. */
  export type IconName = "back" | "close" | "play" | "pause" | "stop" | "check" | "chevron" | "save" | "info";
</script>

<script lang="ts">
  let { name, size = 20 }: { name: IconName; size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#if name === "back"}
    <path d="M12 4.5L6.5 10l5.5 5.5"></path>
  {:else if name === "close"}
    <path d="M5.5 5.5l9 9M14.5 5.5l-9 9"></path>
  {:else if name === "play"}
    <path d="M6.5 4.6v10.8l9-5.4z" fill="currentColor" stroke="none"></path>
  {:else if name === "pause"}
    <rect x="5.4" y="4.6" width="3.4" height="10.8" rx="0.9" fill="currentColor" stroke="none"></rect>
    <rect x="11.2" y="4.6" width="3.4" height="10.8" rx="0.9" fill="currentColor" stroke="none"></rect>
  {:else if name === "stop"}
    <rect x="5.5" y="5.5" width="9" height="9" rx="1.5" fill="currentColor" stroke="none"></rect>
  {:else if name === "check"}
    <path d="M4 10.5l4 3.8L16 5.8" stroke-width="1.8"></path>
  {:else if name === "chevron"}
    <path d="M5 8l5 5 5-5"></path>
  {:else if name === "save"}
    <path d="M10 3.5v9M6 8.8l4 4 4-4M4.5 16h11"></path>
  {:else if name === "info"}
    <circle cx="10" cy="10" r="7.5"></circle><path d="M10 9v5"></path><circle cx="10" cy="6.3" r="0.6" fill="currentColor"></circle>
  {/if}
</svg>
