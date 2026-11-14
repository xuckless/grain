<script lang="ts">
  import type { Difficulty, OctaveMode, Project, ScoreSettings } from "@workstation/engine-api";
  import ScoreGraph from "./ScoreGraph.svelte";
  import type { ScoreState } from "./state";
  import { celebrate, detailDelay, enter, pulse } from "./motion";

  let {
    project,
    score,
    sampleRate,
    onScore,
    onSeek,
  }: {
    project: Project;
    score: ScoreState;
    sampleRate: number;
    onScore: (settings: ScoreSettings) => void;
    onSeek: (frame: number) => void;
  } = $props();

  let difficulty: Difficulty = $state("Medium");
  let octave: OctaveMode = $state("Fold");
  const difficulties: Difficulty[] = ["Easy", "Medium", "Hard"];
</script>

<section>
  <div class="controls">
    <h2>Performance score</h2>
    <label>
      Difficulty
      <select bind:value={difficulty}>
        {#each difficulties as d (d)}<option value={d}>{d}</option>{/each}
      </select>
    </label>
    <label>
      Octave
      <select bind:value={octave}>
        <option value="Fold">Any octave counts</option>
        <option value="Strict">Exact octave</option>
      </select>
    </label>
    <button onclick={() => onScore({ difficulty, octave })} disabled={score.kind === "scoring" || project.analysis.reference_notes.length === 0}>
      Score a take…
    </button>
  </div>

  <!-- Score states crossfade with overlap (motion rule 16). -->
  <div class="stage">
    {#key score.kind}
      <div class="layer" transition:enter>
        {#if score.kind === "scoring"}
          <p class="muted">Tracking your pitch and aligning it to the reference…</p>
        {:else if score.kind === "failed"}
          <p class="error">{score.message}</p>
        {:else if score.kind === "scored"}
          {@const s = score.report.scores}
          {@const details = [
            ["Pitch", s.pitch.toFixed(0)],
            ["Timing", s.timing.toFixed(0)],
            ["Stability", s.stability.toFixed(0)],
            ["Coverage", s.coverage.toFixed(0)],
            ["Take offset", `${score.report.offset_seconds.toFixed(2)} s`],
          ]}
          <div class="cards">
            <!-- The final score reveal is the one celebratory, overshooting moment (rule 13) and pulses once (rule 8). -->
            <div class="card overall" in:celebrate={{ delay: detailDelay(0) }} use:pulse={score.report}>
              <span>Overall</span><strong>{s.overall.toFixed(0)}</strong>
            </div>
            <!-- Details follow the container in a short stagger (rules 3 and 6). -->
            {#each details as [label, value], i (label)}
              <div class="card" in:enter={{ delay: detailDelay(i + 1) }}><span>{label}</span><strong>{value}</strong></div>
            {/each}
          </div>
          <ScoreGraph {project} report={score.report} onSeek={(seconds) => onSeek(Math.floor(seconds * sampleRate))} />
        {/if}
      </div>
    {/key}
  </div>
</section>

<style>
  section {
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .controls {
    display: flex;
    gap: 12px;
    align-items: center;
    flex-wrap: wrap;
  }
  h2 {
    font-size: 15px;
    margin: 0;
  }
  label {
    display: flex;
    gap: 6px;
    align-items: center;
    color: var(--muted);
  }
  .stage {
    display: grid;
  }
  .layer {
    grid-area: 1 / 1;
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .cards {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .card {
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 8px 12px;
    display: flex;
    flex-direction: column;
    min-width: 80px;
  }
  .card span {
    color: var(--muted);
    font-size: 12px;
  }
  .card strong {
    font-size: 22px;
    font-variant-numeric: tabular-nums;
  }
  .overall strong {
    color: var(--accent);
  }
  .muted {
    color: var(--muted);
  }
  .error {
    color: var(--bad);
  }
</style>
