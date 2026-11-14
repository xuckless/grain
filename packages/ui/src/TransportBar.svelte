<script lang="ts">
  import type { PlayerCommand, Project, TransportSnapshot } from "@workstation/engine-api";
  import { formatTime } from "./state";
  import { iconSwap } from "./motion";

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
  const playing = $derived(snapshot.transport.kind === "Playing");
</script>

<div class="bar">
  <button
    class="toggle"
    aria-label={playing ? "Pause" : "Play"}
    onclick={() => onCommand(playing ? { kind: "Pause" } : { kind: "Play" })}
  >
    <!-- Motion rule 18: the icon swap crossfades with scale + blur. -->
    {#key playing}
      <span class="icon" transition:iconSwap>{playing ? "❚❚" : "▶"}</span>
    {/key}
  </button>
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
  .toggle {
    display: grid;
    width: 40px;
    justify-items: center;
  }
  .icon {
    grid-area: 1 / 1;
  }
  .time {
    font-variant-numeric: tabular-nums;
    min-width: 110px;
  }
  .meta {
    color: var(--muted);
  }
</style>
