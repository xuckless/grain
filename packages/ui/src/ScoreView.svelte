<script lang="ts">
  import type { Difficulty, OctaveMode, Project, ScoreSettings } from "@workstation/engine-api";
  import Grain from "./Grain.svelte";
  import Menu from "./Menu.svelte";
  import ScoreGraph from "./ScoreGraph.svelte";
  import Toggle from "./Toggle.svelte";
  import { services } from "./context";
  import { Breath, PitchTrail } from "./grain";
  import { celebrate, detailDelay, dur, enter } from "./motion";
  import type { ScoreState } from "./state";

  /**
   * The Score view. Before a take: the reference melody plays as a pitch trail (design:
   * Particles → Pitch trail). While scoring: Breath, listening. Scored: the score reveal (the one
   * celebratory moment, rule 13) with a hit burst, detail cards settling in (rule 6), and the graph.
   */
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

  const { burst } = services();

  let difficulty = $state<Difficulty>("Medium");
  let octave = $state<OctaveMode>("Fold");
  const difficulties = [
    { id: "Easy", label: "Easy" },
    { id: "Medium", label: "Medium" },
    { id: "Hard", label: "Hard" },
  ] as const satisfies readonly { id: Difficulty; label: string }[];

  const notes = $derived(project.analysis.reference_notes);
  const duration = $derived(project.frames / project.sample_rate);
  const trail = $derived.by(() => {
    const pitch = project.analysis.reference_pitch;
    return new PitchTrail(
      notes,
      (s) => {
        const f = pitch.frames[Math.round(s / pitch.hop_seconds)];
        return f?.kind === "Voiced" ? f.midi : null;
      },
      duration,
    );
  });
  const listening = new Breath();
  listening.intensity = 1;

  let overall = $state<HTMLElement | undefined>();
  // Burst once as the score lands.
  $effect(() => {
    if (score.kind !== "scored" || !overall) return;
    const el = overall;
    const t = setTimeout(() => burst.fireAt(el), detailDelay(0) + dur.celebrate * 0.5);
    return () => clearTimeout(t);
  });
</script>

<section class="score">
  <div class="controls">
    <Menu label="Difficulty" value={difficulty} options={difficulties} onPick={(d) => (difficulty = d)} />
    <Toggle checked={octave === "Fold"} label="Octave folding" onToggle={(on) => (octave = on ? "Fold" : "Strict")} />
    <span class="spacer"></span>
    <button
      class="press primary big"
      onclick={() => onScore({ difficulty, octave })}
      disabled={score.kind === "scoring" || notes.length === 0}
      title={notes.length === 0 ? "No vocal melody was found in this song" : undefined}
    >
      Score a take…
    </button>
  </div>

  <!-- Score states crossfade with overlap (rule 16). -->
  <div class="stage">
    {#key score.kind}
      <div class="layer" transition:enter>
        {#if score.kind === "none"}
          {#if notes.length > 0}
            <Grain behaviour={trail} style="width: 100%; height: 280px" label="The reference melody as a moving pitch trail" />
          {/if}
          <div class="caption">
            <h2>Sing it back</h2>
            <p class="muted">
              {notes.length > 0
                ? "This is the melody you'll be scored against. Record yourself singing the song, then score the take to see where you were sharp, flat, early or late."
                : "No vocal melody was found in this song, so there's nothing to score against."}
            </p>
          </div>
        {:else if score.kind === "scoring"}
          <Grain behaviour={listening} style="width: 100%; height: 280px" />
          <p class="caption muted">Listening to your take and lining it up with the song</p>
        {:else if score.kind === "failed"}
          <div class="caption">
            <h2>Couldn't score that take</h2>
            <p class="muted">{score.message}</p>
          </div>
        {:else}
          {@const s = score.report.scores}
          {@const details = [
            ["Pitch", s.pitch.toFixed(0)],
            ["Timing", s.timing.toFixed(0)],
            ["Stability", s.stability.toFixed(0)],
            ["Coverage", s.coverage.toFixed(0)],
            ["Take offset", `${score.report.offset_seconds.toFixed(2)} s`],
          ]}
          <div class="result">
            <div class="overall">
              <span class="label">Overall</span>
              <strong class="pixel" bind:this={overall} in:celebrate={{ delay: detailDelay(0) }}>{s.overall.toFixed(0)}</strong>
            </div>
            <div class="cards">
              {#each details as [label, value], i (label)}
                <div class="card" in:enter={{ delay: detailDelay(i + 1), detail: true }}>
                  <span class="label">{label}</span>
                  <span class="value">{value}</span>
                </div>
              {/each}
            </div>
          </div>
          <ScoreGraph {project} report={score.report} onSeek={(seconds) => onSeek(Math.floor(seconds * sampleRate))} />
        {/if}
      </div>
    {/key}
  </div>
</section>

<style>
  .score {
    display: flex;
    flex-direction: column;
    gap: 32px;
  }
  .controls {
    display: flex;
    align-items: center;
    gap: 24px;
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
  }
  .stage {
    display: grid;
  }
  .layer {
    grid-area: 1 / 1;
    display: flex;
    flex-direction: column;
    gap: 24px;
    min-width: 0;
  }
  .caption {
    max-width: 560px;
  }
  h2 {
    margin: 0 0 6px;
    font-size: 20px;
    line-height: 28px;
    font-weight: 500;
  }
  p {
    margin: 0;
  }
  .result {
    display: flex;
    align-items: flex-end;
    gap: 64px;
    flex-wrap: wrap;
    padding-top: 20px;
    border-top: 0.5px solid var(--c-text);
  }
  .overall {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .overall strong {
    display: inline-block;
    font-size: 96px;
    line-height: 96px;
    font-weight: 400;
  }
  .label {
    font-size: 13px;
    line-height: 18px;
    font-weight: 500;
    color: var(--c-muted);
  }
  .cards {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
  }
  .card {
    min-width: 112px;
    box-sizing: border-box;
    padding: 14px 16px;
    background: var(--c-surface);
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .value {
    font-size: 20px;
    line-height: 28px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }
</style>
