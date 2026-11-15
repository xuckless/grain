<script lang="ts">
  import type { EngineHost, TransportSnapshot } from "@workstation/engine-api";
  import BottomAction from "./BottomAction.svelte";
  import Grain from "./Grain.svelte";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";
  import SettingsControls from "./SettingsControls.svelte";
  import ScoreGraph from "./ScoreGraph.svelte";
  import TakePicker from "./TakePicker.svelte";
  import Toolbar, { type OverflowItem } from "./Toolbar.svelte";
  import Takes from "./Takes.svelte";
  import { selectedTakeId } from "./chain";
  import { listener, services } from "./context";
  import { Breath, PitchTrail } from "./grain";
  import { celebrate, detailDelay, dur, enter } from "./motion";
  import { applyTake } from "./recorder";
  import { errorMessage, type Session } from "./state";
  import { isListening, LISTEN_MODES, noteSpan } from "./listen";
  import { pitchFeedback } from "./feedback";
  import { noteAt } from "./karaoke";

  /**
   * The Score view (AGENTS.md §5, §8.6): the score of the selected take (the ★ master unless
   * another is picked; takes are sung in Karaoke or imported from ⋯). Before a score: the
   * reference melody plays as a pitch trail (design: Particles → Pitch trail). While scoring:
   * Breath. Scored: the score reveal (the one celebratory moment, rule 13) with a hit burst,
   * detail cards settling in (rule 6), and the graph, whose notes play A/B. Scores measure the
   * singing: the raw take, before its processing chain.
   */
  let {
    host,
    session,
    snapshot,
    onSeek,
  }: {
    host: EngineHost;
    session: Session;
    snapshot: TransportSnapshot;
    onSeek: (frame: number) => void;
  } = $props();

  const { burst } = services();

  /** Copied: the session's settings are a reactive proxy, which hosts can't post or clone. */
  const settings = () => ({ ...session.settings });

  const project = $derived(session.project);
  const score = $derived(session.score);
  const notes = $derived(project.analysis.reference_notes);
  const duration = $derived(project.frames / project.sample_rate);
  // The reference melody, looping from when the view opened.
  const trail = $derived.by(() => {
    const pitch = project.analysis.reference_pitch;
    const t0 = performance.now();
    return new PitchTrail(
      notes,
      (now) => (((now - t0) / 1000) % duration + duration) % duration,
      0.7,
      (s) => {
        const f = pitch.frames[Math.round(s / pitch.hop_seconds)];
        if (f?.kind !== "Voiced") return null;
        const n = noteAt(notes, s);
        return { seconds: s, midi: f.midi, feedback: pitchFeedback(f.midi, n < 0 ? null : notes[n].midi) };
      },
    );
  });
  const listening = new Breath();
  listening.intensity = 1;

  // ---- Scoring the selected take --------------------------------------------------------------

  const take = $derived(selectedTakeId(project, session.take));

  async function rescore(id: number) {
    session.score = { kind: "scoring" };
    try {
      session.score = { kind: "scored", take: id, report: await host.scoreRecorded(id, settings()) };
    } catch (e) {
      session.score = { kind: "failed", message: errorMessage(e) };
    }
  }

  // The score follows the take picker.
  $effect(() => {
    const id = take;
    const s = session.score;
    if (id === null || s.kind === "scoring" || (s.kind === "scored" && s.take === id)) return;
    void rescore(id);
  });

  function settingsChanged() {
    if (score.kind === "scored") void rescore(score.take);
  }

  async function importTake() {
    listen({ kind: "stop" });
    const before = session.score;
    session.score = { kind: "scoring" };
    try {
      const outcome = await host.scoreTake(settings());
      if (outcome.kind === "scored") {
        applyTake(session, outcome.added);
        session.take = outcome.added.take;
      } else session.score = before;
    } catch (e) {
      session.score = { kind: "failed", message: errorMessage(e) };
    }
  }

  // ---- Listening (the app's listen machine) ----------------------------------------------------

  const listen = listener();
  const auditioning = $derived(session.listening.kind !== "idle" && session.listening.span !== null ? session.listening.item.kind : null);
  const playingTake = $derived(take !== null && session.listening.kind === "playing" && isListening(session.listening, { kind: "Take", id: take }));

  function playTake() {
    if (take === null) return;
    if (playingTake) listen({ kind: "pause" });
    else listen({ kind: "play", items: [{ kind: "Take", id: take }], mode: session.playback });
  }

  let overall = $state<HTMLElement | undefined>();
  // Burst once as the score lands.
  $effect(() => {
    if (score.kind !== "scored" || !overall) return;
    const el = overall;
    const t = setTimeout(() => burst.fireAt(el), detailDelay(0) + dur.celebrate * 0.5);
    return () => clearTimeout(t);
  });

  const busy = $derived(score.kind === "scoring");
  const stage = $derived(score.kind);
  const overflow = $derived<OverflowItem[]>([{ label: "Import a take…", run: importTake, disabled: busy ? "Scoring" : notes.length === 0 ? "No vocal melody was found in this song" : null }]);
  const hints = [
    "Scores measure your singing: the take as recorded, before its pitch edits, auto-tune and production.",
    "Click a note in the graph to hear the original, then your take, over that note.",
  ];
</script>

<section class="score">
  <Toolbar {hints} {overflow}>
    <TakePicker {host} {session} />
    <Menu label="Play" value={session.playback} options={LISTEN_MODES} onPick={(m) => (session.playback = m)} />
    <SettingsControls {session} onChange={settingsChanged} />
  </Toolbar>

  <!-- Score states crossfade with overlap (rule 16). -->
  <div class="stage">
    {#key stage}
      <div class="layer" transition:enter>
        {#if stage === "none"}
          {#if notes.length > 0}
            <Grain behaviour={trail} style="width: 100%; height: 280px" label="The reference melody as a moving pitch trail" />
          {/if}
          <div class="caption">
            <h2>Sing it back</h2>
            <p class="muted">
              {notes.length > 0
                ? "This is the melody you'll be scored against. Sing a take in Karaoke, or import one from ⋯, to see where you were sharp, flat, early or late."
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
        {:else if score.kind === "scored"}
          {@const s = score.report.scores}
          {@const take = project.takes.find((t) => t.id === score.take)}
          {@const details = [
            ["Pitch", s.pitch.toFixed(0)],
            ["Timing", s.timing.toFixed(0)],
            ["Stability", s.stability.toFixed(0)],
            ["Coverage", s.coverage.toFixed(0)],
            take?.source.kind === "Recorded"
              ? ["Latency removed", `${take.source.latency_ms.toFixed(0)} ms`]
              : ["Take starts", `${(take?.start_seconds ?? 0).toFixed(2)} s`],
            ["Timing drift", `${score.report.timing_deviation.mean_ms >= 0 ? "+" : ""}${score.report.timing_deviation.mean_ms.toFixed(0)} ms avg, ${score.report.timing_deviation.max_abs_ms.toFixed(0)} max`],
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
          <ScoreGraph
            {project}
            report={score.report}
            onNote={(start, end) => listen({ kind: "play", items: [{ kind: "VocalStem" }, { kind: "Take", id: score.take }], mode: "Voice", span: noteSpan(start, end) })}
            onSeek={(seconds) => onSeek(Math.floor(seconds * snapshot.sample_rate))}
          />
          {#if auditioning}
            <p class="muted">{auditioning === "VocalStem" ? "A: the original vocal" : "B: your take"}</p>
          {/if}
        {/if}
      </div>
    {/key}
  </div>

  <Takes {host} {session} onUse={(id) => !busy && (session.take = id)} empty="Sing a take in Karaoke or import one: it's kept with the song." />

  {#if take !== null}
    <BottomAction>
      <button class="press cta" onclick={playTake}><Icon name={playingTake ? "pause" : "play"} />{playingTake ? "Pause" : "Play take"}</button>
    </BottomAction>
  {/if}
</section>

<style>
  .score {
    display: flex;
    flex-direction: column;
    gap: 32px;
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
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h2 {
    margin: 0;
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
