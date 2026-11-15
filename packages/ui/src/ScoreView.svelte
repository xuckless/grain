<script lang="ts">
  import type { EngineHost, PlayerCommand, TransportSnapshot } from "@workstation/engine-api";
  import Grain from "./Grain.svelte";
  import Menu from "./Menu.svelte";
  import RecordingSetup from "./RecordingSetup.svelte";
  import SettingsControls from "./SettingsControls.svelte";
  import ScoreGraph from "./ScoreGraph.svelte";
  import { services } from "./context";
  import { Breath, dotMeter, fit, PitchTrail } from "./grain";
  import { celebrate, detailDelay, dur, enter } from "./motion";
  import { applyTake, armTake, cancelTake, eachFrame, effectiveBacking, finishTake, runCommands } from "./recorder";
  import { errorMessage, type Session } from "./state";
  import { beatsLeft, isCapturing, observeRecording, recordingNext, type Recording, type RecordingEvent } from "./takes";
  import Takes from "./Takes.svelte";
  import { listener } from "./context";
  import { noteSpan, sungTakes } from "./listen";
  import { colour, stemHue } from "./theme";
  import { pitchFeedback } from "./feedback";
  import { noteAt } from "./karaoke";

  /**
   * The Score view. Before a take: the reference melody plays as a pitch trail (design:
   * Particles → Pitch trail). Recording: the count-in, then Breath listening with an input meter.
   * While scoring: Breath. Scored: the score reveal (the one celebratory moment, rule 13) with a
   * hit burst, detail cards settling in (rule 6), and the graph, whose notes play A/B.
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
  // The host is fixed for the app's lifetime.
  // svelte-ignore state_referenced_locally
  const mic = host.microphone;

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

  // ---- Commands -------------------------------------------------------------------------------

  const run = (commands: PlayerCommand[]) => runCommands(host, commands);

  async function rescore(take: number) {
    session.score = { kind: "scoring" };
    try {
      session.score = { kind: "scored", take, report: await host.scoreRecorded(take, settings()) };
    } catch (e) {
      session.score = { kind: "failed", message: errorMessage(e) };
    }
  }

  function settingsChanged() {
    if (score.kind === "scored") void rescore(score.take);
  }

  async function importTake() {
    await stopAudition();
    const before = session.score;
    session.score = { kind: "scoring" };
    try {
      const outcome = await host.scoreTake(settings());
      if (outcome.kind === "scored") applyTake(session, outcome.added);
      else session.score = before;
    } catch (e) {
      session.score = { kind: "failed", message: errorMessage(e) };
    }
  }

  // ---- Recording ------------------------------------------------------------------------------

  let recording = $state<Recording>({ kind: "idle" });
  let level = $state(0);

  function recordingEvent(e: RecordingEvent): boolean {
    const next = recordingNext(recording, e);
    if (!next) return false;
    recording = next;
    return true;
  }

  async function record() {
    if (mic.kind !== "available" || !recordingEvent({ kind: "arm" })) return;
    await stopAudition();
    try {
      await armTake(host, project, effectiveBacking(session));
    } catch (e) {
      recordingEvent({ kind: "fail", message: errorMessage(e) });
    }
  }

  async function stopRecording() {
    if (mic.kind !== "available" || !recordingEvent({ kind: "stop" })) return;
    session.score = { kind: "scoring" };
    try {
      const added = await finishTake(host, project, settings());
      applyTake(session, added);
      recordingEvent({ kind: "captured", take: added.take });
    } catch (e) {
      session.score = { kind: "failed", message: errorMessage(e) };
      recordingEvent({ kind: "fail", message: errorMessage(e) });
    }
  }

  async function cancelRecording() {
    if (mic.kind !== "available" || !recordingEvent({ kind: "cancel" })) return;
    await cancelTake(host, project);
  }

  // The transport ends the count-in, and stopping the song (or its end) stops the take.
  $effect(() => {
    const e = observeRecording(recording, snapshot.transport);
    if (!e) return;
    if (e.kind === "stop") void stopRecording();
    else recordingEvent(e);
  });

  // Input level while the microphone is open.
  $effect(() => {
    if (!isCapturing(recording) || mic.kind !== "available") return;
    const stop = eachFrame(async () => {
      level = await mic.level().catch(() => 0);
      listening.intensity = 0.35 + 0.65 * Math.min(1, level * 4);
    });
    return () => {
      stop();
      level = 0;
      listening.intensity = 1;
    };
  });

  let meter = $state<HTMLCanvasElement | undefined>();
  const METER_W = 240;
  const METER_H = 12;
  $effect(() => {
    if (!meter) return;
    const c = fit(meter, METER_W, METER_H);
    if (c) dotMeter(c, METER_W, METER_H, level, stemHue.Vocals, colour.disabled);
  });

  const beats = $derived(beatsLeft(snapshot.transport, project.analysis.tempo.bpm, snapshot.sample_rate));

  let calibrating = $state(false);

  // ---- A/B audition (the app's listen machine: the original's note, then the take's) ---------

  const listen = listener();
  /** Recording or calibrating takes the player: whatever is being heard stops first. */
  async function stopAudition() {
    listen({ kind: "stop" });
  }
  const auditioning = $derived(session.listening.kind !== "idle" && session.listening.span !== null ? session.listening.item.kind : null);

  // ---- Takes ----------------------------------------------------------------------------------

  /** "" until a take is picked (a reopened song shows no score yet). */
  const takes = $derived([...(score.kind === "scored" ? [] : [{ id: "", label: "Pick a take" }]), ...sungTakes(project).map((t) => ({ id: String(t.id), label: t.name }))]);
  const shown = $derived(score.kind === "scored" ? String(score.take) : "");

  let overall = $state<HTMLElement | undefined>();
  // Burst once as the score lands.
  $effect(() => {
    if (score.kind !== "scored" || !overall) return;
    const el = overall;
    const t = setTimeout(() => burst.fireAt(el), detailDelay(0) + dur.celebrate * 0.5);
    return () => clearTimeout(t);
  });

  const busy = $derived(isCapturing(recording) || score.kind === "scoring" || calibrating);
  const stage = $derived(isCapturing(recording) ? "recording" : score.kind);
</script>

<section class="score">
  <div class="controls">
    <SettingsControls {session} onChange={settingsChanged} />
    {#if takes.length > (score.kind === "scored" ? 0 : 1)}
      <Menu label="Take" value={shown} options={takes} onPick={(id) => id !== "" && !busy && void rescore(Number(id))} />
    {/if}
    <span class="spacer"></span>
    <button class="press raised big" onclick={importTake} disabled={busy || notes.length === 0}>Import take…</button>
    {#if isCapturing(recording)}
      <button class="press raised big" onclick={cancelRecording} disabled={recording.kind === "stopping"}>Cancel</button>
      <button class="press primary big" onclick={stopRecording} disabled={recording.kind === "stopping"}>Stop</button>
    {:else}
      <button
        class="press primary big"
        onclick={record}
        disabled={busy || notes.length === 0 || mic.kind !== "available"}
        title={mic.kind === "unavailable" ? mic.reason : notes.length === 0 ? "No vocal melody was found in this song" : undefined}
      >
        Record a take
      </button>
    {/if}
  </div>

  <div class="setup">
    <RecordingSetup {host} {session} busy={isCapturing(recording) || score.kind === "scoring"} bind:calibrating beforeCalibrate={stopAudition} />
  </div>

  <!-- Score states crossfade with overlap (rule 16). -->
  <div class="stage">
    {#key stage}
      <div class="layer" transition:enter>
        {#if stage === "recording"}
          <Grain behaviour={listening} style="width: 100%; height: 280px" label="Listening" />
          <div class="caption">
            {#if recording.kind === "countIn"}
              <strong class="pixel countdown">{beats || ""}</strong>
              <p class="muted">Count-in: sing from the next bar.</p>
            {:else if recording.kind === "stopping"}
              <p class="muted">Lining your take up with the song</p>
            {:else}
              <h2>Recording</h2>
              <p class="muted">Sing along. Stop when you're done, or let the song finish.</p>
            {/if}
            <canvas bind:this={meter} class="meter" style:width="{METER_W}px" style:height="{METER_H}px" aria-label="Input level"></canvas>
            <p class="muted">Use headphones, so the backing doesn't bleed into your take.</p>
          </div>
        {:else if stage === "none"}
          {#if notes.length > 0}
            <Grain behaviour={trail} style="width: 100%; height: 280px" label="The reference melody as a moving pitch trail" />
          {/if}
          <div class="caption">
            <h2>Sing it back</h2>
            <p class="muted">
              {notes.length > 0
                ? "This is the melody you'll be scored against. Record a take over the song, or import one, to see where you were sharp, flat, early or late."
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
            onNote={(start, end) => listen({ kind: "play", items: [{ kind: "VocalStem" }, { kind: "Take", id: score.take }], mode: "Solo", span: noteSpan(start, end) })}
            onSeek={(seconds) => onSeek(Math.floor(seconds * snapshot.sample_rate))}
          />
          {#if auditioning}
            <p class="muted">{auditioning === "VocalStem" ? "A: the original vocal" : "B: your take"}</p>
          {/if}
        {/if}
      </div>
    {/key}
  </div>

  {#if !isCapturing(recording)}
    <Takes {host} {session} onUse={(id) => !busy && void rescore(id)} empty="Record a take or import one: it's kept with the song." />
  {/if}
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
  .setup {
    margin-top: -12px;
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
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .countdown {
    font-size: 64px;
    line-height: 64px;
    font-weight: 400;
    min-height: 64px;
  }
  .meter {
    display: block;
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
