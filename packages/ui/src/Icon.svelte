<script lang="ts" module>
  /** Every icon in the UI: inline stroke SVGs on a 20×20 grid, coloured by `currentColor`. */
  export type IconName =
    | "back" | "close" | "play" | "pause" | "stop" | "check" | "chevron" | "save" | "info"
    | "gear" | "more" | "star" | "starFilled" | "undo" | "redo" | "export" | "zoomIn" | "zoomOut" | "fit"
    | "mute" | "unmute" | "solo" | "midi" | "loop" | "record" | "trash" | "retake"
    | "mic" | "melody" | "drums" | "bass" | "guitar" | "piano" | "waves";
</script>

<script lang="ts">
  let { name, size = 20 }: { name: IconName; size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
  {#if name === "back"}
    <path d="M12 4.5L6.5 10l5.5 5.5"></path>
  {:else if name === "trash"}
    <path d="M4.5 6h11M8 6V4.5h4V6M6 6l.7 9.5h6.6L14 6M8.5 8.5v4.5M11.5 8.5v4.5"></path>
  {:else if name === "retake"}
    <path d="M15 10a5 5 0 1 1-1.5-3.6M13.8 3.8v2.9h-2.9"></path><circle cx="10" cy="10" r="1.6" fill="currentColor" stroke="none"></circle>
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
  {:else if name === "gear"}
    <path d="M8.58 4.69L8.84 2.69L11.16 2.69L11.42 4.69A5.5 5.5 0 0 1 12.75 5.24L14.35 4.01L15.99 5.65L14.76 7.25A5.5 5.5 0 0 1 15.31 8.58L17.31 8.84L17.31 11.16L15.31 11.42A5.5 5.5 0 0 1 14.76 12.75L15.99 14.35L14.35 15.99L12.75 14.76A5.5 5.5 0 0 1 11.42 15.31L11.16 17.31L8.84 17.31L8.58 15.31A5.5 5.5 0 0 1 7.25 14.76L5.65 15.99L4.01 14.35L5.24 12.75A5.5 5.5 0 0 1 4.69 11.42L2.69 11.16L2.69 8.84L4.69 8.58A5.5 5.5 0 0 1 5.24 7.25L4.01 5.65L5.65 4.01L7.25 5.24A5.5 5.5 0 0 1 8.58 4.69Z"></path><circle cx="10" cy="10" r="2.3"></circle>
  {:else if name === "more"}
    <circle cx="4.5" cy="10" r="1.2" fill="currentColor" stroke="none"></circle><circle cx="10" cy="10" r="1.2" fill="currentColor" stroke="none"></circle><circle cx="15.5" cy="10" r="1.2" fill="currentColor" stroke="none"></circle>
  {:else if name === "star" || name === "starFilled"}
    <path d="M10 3.2l2 4.3 4.6.5-3.4 3.1 1 4.6L10 13.4l-4.2 2.3 1-4.6L3.4 8l4.6-.5z" fill={name === "starFilled" ? "currentColor" : "none"}></path>
  {:else if name === "undo"}
    <path d="M7 6.5L4 9.5l3 3"></path><path d="M4.5 9.5h7a4 4 0 010 8H9"></path>
  {:else if name === "redo"}
    <path d="M13 6.5l3 3-3 3"></path><path d="M15.5 9.5h-7a4 4 0 000 8H11"></path>
  {:else if name === "export"}
    <path d="M10 13V3.5M6 7.3l4-4 4 4M4.5 12.5v4h11v-4"></path>
  {:else if name === "zoomIn" || name === "zoomOut"}
    <circle cx="8.5" cy="8.5" r="5"></circle><path d="M12.2 12.2l4.3 4.3M6.3 8.5h4.4"></path>{#if name === "zoomIn"}<path d="M8.5 6.3v4.4"></path>{/if}
  {:else if name === "fit"}
    <path d="M3.5 7V3.5H7M13 3.5h3.5V7M16.5 13v3.5H13M7 16.5H3.5V13"></path>
  {:else if name === "mute" || name === "unmute"}
    <path d="M3.5 7.8h2.8L10 4.5v11l-3.7-3.3H3.5z"></path>{#if name === "mute"}<path d="M13 7.5l4 5M17 7.5l-4 5"></path>{:else}<path d="M13 7.3a4 4 0 010 5.4M15.2 5.3a7 7 0 010 9.4"></path>{/if}
  {:else if name === "solo"}
    <path d="M4 12.5v-2a6 6 0 0112 0v2"></path><rect x="3.2" y="11.8" width="3.3" height="5" rx="1.2"></rect><rect x="13.5" y="11.8" width="3.3" height="5" rx="1.2"></rect>
  {:else if name === "midi"}
    <rect x="3" y="4.5" width="14" height="11" rx="1.5"></rect><path d="M7.2 4.5v6.5M10 4.5v6.5M12.8 4.5v6.5"></path>
  {:else if name === "loop"}
    <path d="M4 8.5V7.5a2.5 2.5 0 012.5-2.5H15M12.8 2.8L15 5l-2.2 2.2M16 11.5v1a2.5 2.5 0 01-2.5 2.5H5M7.2 17.2L5 15l2.2-2.2"></path>
  {:else if name === "record"}
    <circle cx="10" cy="10" r="5.5" fill="currentColor" stroke="none"></circle>
  {:else if name === "melody"}
    <ellipse cx="6" cy="14.6" rx="2.3" ry="1.8"></ellipse><path d="M8.3 14.4V4.6l7.2-1.6v9.6"></path><ellipse cx="13.2" cy="12.8" rx="2.3" ry="1.8"></ellipse><path d="M8.3 7.4l7.2-1.6"></path>
  {:else if name === "mic"}
    <rect x="7.5" y="2.8" width="5" height="9" rx="2.5"></rect><path d="M4.8 9.6a5.2 5.2 0 0010.4 0M10 14.8v2.6M7.2 17.4h5.6"></path>
  {:else if name === "drums"}
    <ellipse cx="10" cy="8" rx="6.5" ry="2.3"></ellipse><path d="M3.5 8v5.5c0 1.3 2.9 2.3 6.5 2.3s6.5-1 6.5-2.3V8M5.5 3.2l3 3.6M14.5 3.2l-3 3.6"></path>
  {:else if name === "bass"}
    <path d="M15.8 2.6l1.6 1.6-5.4 5.4"></path><path d="M12 9.6l-1.6-1.6-4.6 1.7a3.2 3.2 0 00-1 5.3 3.2 3.2 0 005.3-1z"></path><circle cx="7.6" cy="12.4" r="0.8" fill="currentColor" stroke="none"></circle>
  {:else if name === "guitar"}
    <path d="M14.6 2.8l2.6 2.6-1.4 1.4-2.6-2.6zM13.2 6.8l-3 3"></path><path d="M10.4 8.4c-1.2-.6-2.8-.4-3.6.6-.6.6-.5 1.4-1.4 1.8-1.4.5-2.4 1.6-2 3.2.3 1.3 1.4 2.4 2.8 2.7 1.5.3 2.6-.6 3.1-2 .4-.9 1.2-.8 1.8-1.4 1-.8 1.2-2.4.6-3.6"></path>
  {:else if name === "piano"}
    <rect x="3" y="4" width="14" height="12" rx="1.5"></rect><path d="M6.5 4v12M10 4v12M13.5 4v12"></path><path d="M5.6 4h1.8v6.5H5.6zM9.1 4h1.8v6.5H9.1zM12.6 4h1.8v6.5h-1.8z" fill="currentColor" stroke="none"></path>
  {:else if name === "waves"}
    <path d="M2.8 8c1.8-2 3.4-2 5 0s3.4 2 5 0 3.4-2 4.4-1M2.8 12.5c1.8-2 3.4-2 5 0s3.4 2 5 0 3.4-2 4.4-1"></path>
  {:else if name === "info"}
    <circle cx="10" cy="10" r="7.5"></circle><path d="M10 9v5"></path><circle cx="10" cy="6.3" r="0.6" fill="currentColor"></circle>
  {/if}
</svg>
