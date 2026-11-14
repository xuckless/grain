<script lang="ts">
  import type { PlayerCommand, Project, TransportSnapshot } from "@workstation/engine-api";
  import { formatTime } from "./state";

  let { project, snapshot, onCommand }: { project: Project; snapshot: TransportSnapshot; onCommand: (c: PlayerCommand) => void } = $props();

  const position = $derived.by(() => {
    const t = snapshot.transport;
    switch (t.kind) {
      case "Stopped":
        return 0;
      case "Playing":
      case "Paused":
        return t.frame / snapshot.sample_rate;
    }
  });
  const duration = $derived(project.frames / project.sample_rate);
  const key = $derived(project.analysis.keys[0]);
</script>

<div class="bar">
  {#if snapshot.transport.kind === "Playing"}
    <button onclick={() => onCommand({ kind: "Pause" })}>Pause</button>
  {:else}
    <button onclick={() => onCommand({ kind: "Play" })}>Play</button>
  {/if}
  <button onclick={() => onCommand({ kind: "Stop" })} disabled={snapshot.transport.kind === "Stopped"}>Stop</button>
  <span class="time">{formatTime(position)} / {formatTime(duration)}</span>
  <span class="meta">{project.analysis.tempo.bpm.toFixed(1)} BPM</span>
  {#if key}
    <span class="meta">{key.label}</span>
  {/if}
  <span class="meta">{project.song.kind === "File" ? project.song.name : project.song.title}</span>
</div>

<style>
  .bar {
    display: flex;
    gap: 10px;
    align-items: center;
    flex-wrap: wrap;
  }
  .time {
    font-variant-numeric: tabular-nums;
    min-width: 110px;
  }
  .meta {
    color: var(--muted);
  }
</style>
