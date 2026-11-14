<script lang="ts">
  import type { Difficulty, EngineHost, Latency, OctaveMode, PlayerCommand, ScoreSettings, TakeAdded, TransportSnapshot } from "@workstation/engine-api";
  import Grain from "./Grain.svelte";
  import Menu from "./Menu.svelte";
  import ScoreGraph from "./ScoreGraph.svelte";
  import Toggle from "./Toggle.svelte";
  import { services } from "./context";
  import { Breath, dotMeter, fit, PitchTrail } from "./grain";
  import { celebrate, detailDelay, dur, enter } from "./motion";
  import { errorMessage, type Session } from "./state";
  import {
    auditionCommands,
    auditionEndCommands,
    auditionNext,
    backingCommands,
    backingNote,
    backingOptions,
    backingRestoreCommands,
    beatsLeft,
    calibrationNext,
    defaultBacking,
    isCapturing,
    LATENCY_NUDGE_MS,
    noteSpan,
    observeAudition,
    observeRecording,
    recordingNext,
    type Audition,
    type AuditionEvent,
    type Backing,
    type Calibration,
    type CalibrationEvent,
    type Recording,
    type RecordingEvent,
  } from "./takes";
  import { colour, stemHue } from "./theme";

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

  const { burst, toast } = services();
  // The host is fixed for the app's lifetime.
  // svelte-ignore state_referenced_locally
  const mic = host.microphone;

  let difficulty = $state<Difficulty>("Medium");
  let octave = $state<OctaveMode>("Fold");
  const difficulties = [
    { id: "Easy", label: "Easy" },
    { id: "Medium", label: "Medium" },
    { id: "Hard", label: "Hard" },
  ] as const satisfies readonly { id: Difficulty; label: string }[];
  const settings = (): ScoreSettings => ({ difficulty, octave });

  const project = $derived(session.project);
  const score = $derived(session.score);
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

  // ---- Commands -------------------------------------------------------------------------------

  /** Sends commands in order. The engine rejects illegal ones; the transport poll shows the truth. */
  async function run(commands: PlayerCommand[]) {
    for (const command of commands) {
      try {
        await host.send(command);
      } catch (e) {
        console.warn(errorMessage(e));
      }
    }
  }

  /** A take was added: it becomes the shown, scored take and the song has unsaved changes. */
  function taken(added: TakeAdded) {
    session.project = added.project;
    session.score = { kind: "scored", take: added.take, report: added.report };
    if (session.save.kind === "clean") session.save = { kind: "dirty" };
  }

  async function rescore(take: number) {
    session.score = { kind: "scoring" };
    try {
      session.score = { kind: "scored", take, report: await host.scoreRecorded(take, settings()) };
    } catch (e) {
      session.score = { kind: "failed", message: errorMessage(e) };
    }
  }

  function setSettings(next: () => void) {
    next();
    if (score.kind === "scored") void rescore(score.take);
  }

  async function importTake() {
    await stopAudition();
    const before = session.score;
    session.score = { kind: "scoring" };
    try {
      const outcome = await host.scoreTake(settings());
      if (outcome.kind === "scored") taken(outcome.added);
      else session.score = before;
    } catch (e) {
      session.score = { kind: "failed", message: errorMessage(e) };
    }
  }

  // ---- Recording ------------------------------------------------------------------------------

  const backings = $derived(backingOptions(project));
  /** Decided once per song from the stem levels measured at import or load. */
  const suggested = $derived(defaultBacking(project.analysis.accompaniment));
  let chosen = $state<Backing | null>(null);
  const backing = $derived(chosen ?? suggested);
  const note = $derived(backing === suggested ? backingNote(project.analysis.accompaniment) : null);

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
    await run(backingCommands(project, backing));
    try {
      await mic.start();
    } catch (e) {
      await run(backingRestoreCommands(project));
      recordingEvent({ kind: "fail", message: errorMessage(e) });
    }
  }

  async function stopRecording() {
    if (mic.kind !== "available" || !recordingEvent({ kind: "stop" })) return;
    // Put the vocal back before the host snapshots the project into the new take's result.
    await run(backingRestoreCommands(project));
    session.score = { kind: "scoring" };
    try {
      const added = await mic.finish(settings());
      taken(added);
      recordingEvent({ kind: "captured", take: added.take });
    } catch (e) {
      session.score = { kind: "failed", message: errorMessage(e) };
      recordingEvent({ kind: "fail", message: errorMessage(e) });
    }
  }

  async function cancelRecording() {
    if (mic.kind !== "available" || !recordingEvent({ kind: "cancel" })) return;
    await mic.cancel();
    await run(backingRestoreCommands(project));
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
    let alive = true;
    const poll = async () => {
      if (!alive) return;
      level = await mic.level().catch(() => 0);
      listening.intensity = 0.35 + 0.65 * Math.min(1, level * 4);
      requestAnimationFrame(poll);
    };
    requestAnimationFrame(poll);
    return () => {
      alive = false;
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

  // ---- Latency calibration --------------------------------------------------------------------

  let calibration = $state<Calibration>({ kind: "idle" });
  let latency = $state<Latency | null>(null);
  $effect(() => {
    if (mic.kind === "available") mic.latency().then((l) => (latency = l), (e) => console.warn(errorMessage(e)));
  });

  function calibrationEvent(e: CalibrationEvent): boolean {
    const next = calibrationNext(calibration, e);
    if (!next) return false;
    calibration = next;
    return true;
  }

  async function calibrate() {
    if (mic.kind !== "available" || isCapturing(recording) || !calibrationEvent({ kind: "start" })) return;
    await stopAudition();
    try {
      latency = await mic.measureLatency();
      calibrationEvent({ kind: "done", latency });
    } catch (e) {
      calibrationEvent({ kind: "fail", message: errorMessage(e) });
    }
  }

  async function nudge(ms: number) {
    if (mic.kind !== "available" || !latency) return;
    try {
      latency = await mic.setLatency(latency.ms + ms);
    } catch (e) {
      toast(`Couldn't change the latency: ${errorMessage(e)}`, "error");
    }
  }

  function describeLatency(l: Latency): string {
    switch (l.source.kind) {
      case "Default":
        return "estimated, not calibrated";
      case "Measured":
        return "measured";
      case "Manual":
        return "set by hand";
    }
  }

  // ---- A/B audition ---------------------------------------------------------------------------

  let audition = $state<Audition>({ kind: "idle" });

  async function auditionEvent(e: AuditionEvent) {
    const previous = audition;
    const next = auditionNext(audition, e);
    if (!next) return;
    audition = next;
    if (next.kind === "idle") await run(auditionEndCommands(project, previous));
    else if (next.kind !== previous.kind || e.kind === "start") await run(auditionCommands(project, next, snapshot.sample_rate));
  }

  async function stopAudition() {
    await auditionEvent({ kind: "cancel" });
  }

  $effect(() => {
    const e = observeAudition(audition, snapshot.transport, snapshot.sample_rate);
    if (e) void auditionEvent(e);
  });

  // ---- Takes ----------------------------------------------------------------------------------

  /** "" until a take is picked (a reopened song shows no score yet). */
  const takes = $derived([...(score.kind === "scored" ? [] : [{ id: "", label: "Pick a take" }]), ...project.takes.map((t) => ({ id: String(t.id), label: t.name }))]);
  const shown = $derived(score.kind === "scored" ? String(score.take) : "");

  let overall = $state<HTMLElement | undefined>();
  // Burst once as the score lands.
  $effect(() => {
    if (score.kind !== "scored" || !overall) return;
    const el = overall;
    const t = setTimeout(() => burst.fireAt(el), detailDelay(0) + dur.celebrate * 0.5);
    return () => clearTimeout(t);
  });

  const busy = $derived(isCapturing(recording) || score.kind === "scoring" || calibration.kind === "measuring");
  const stage = $derived(isCapturing(recording) ? "recording" : score.kind);
</script>

<section class="score">
  <div class="controls">
    <Menu label="Difficulty" value={difficulty} options={difficulties} onPick={(d) => setSettings(() => (difficulty = d))} />
    <Toggle checked={octave === "Fold"} label="Octave folding" onToggle={(on) => setSettings(() => (octave = on ? "Fold" : "Strict"))} />
    {#if project.takes.length > 0}
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

  {#if mic.kind === "available"}
    <div class="setup">
      <div class="row">
        <Menu
          label="Backing"
          value={backing}
          options={backings.filter((b) => b.available)}
          onPick={(b) => (chosen = b)}
        />
        {#if note}<span class="muted">{note}</span>{/if}
      </div>
      <div class="row">
        <span class="label">Latency</span>
        {#if latency}
          <button class="press icon quiet" aria-label="{LATENCY_NUDGE_MS} ms less" disabled={busy} onclick={() => nudge(-LATENCY_NUDGE_MS)}>−</button>
          <span class="value-sm">{latency.ms.toFixed(0)} ms</span>
          <button class="press icon quiet" aria-label="{LATENCY_NUDGE_MS} ms more" disabled={busy} onclick={() => nudge(LATENCY_NUDGE_MS)}>+</button>
          <span class="muted">{describeLatency(latency)}</span>
        {/if}
        <button class="press raised calibrate" onclick={calibrate} disabled={busy}>
          {calibration.kind === "measuring" ? "Listening…" : "Calibrate"}
        </button>
      </div>
      <p class="muted hint">
        {#if calibration.kind === "measuring"}
          Playing a short chirp. Keep still and quiet.
        {:else if calibration.kind === "failed"}
          {calibration.message}
        {:else if calibration.kind === "measured"}
          Measured {calibration.latency.ms.toFixed(0)} ms. Plug your headphones back in to record.
        {:else}
          Calibrate once per microphone and output: unplug your headphones, or hold them against the microphone, so it can
          hear the test chirp.
        {/if}
      </p>
    </div>
  {/if}

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
            onNote={(start, end) => void auditionEvent({ kind: "start", span: noteSpan(start, end), take: score.take })}
            onSeek={(seconds) => onSeek(Math.floor(seconds * snapshot.sample_rate))}
          />
          {#if audition.kind !== "idle"}
            <p class="muted">{audition.kind === "reference" ? "A: the original vocal" : "B: your take"}</p>
          {/if}
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
  .setup {
    display: flex;
    flex-direction: column;
    gap: 12px;
    margin-top: -12px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .calibrate {
    height: 36px;
    padding: 0 14px;
    border-radius: 10px;
    margin-left: 12px;
  }
  .value-sm {
    min-width: 56px;
    text-align: center;
    font-variant-numeric: tabular-nums;
    font-weight: 500;
  }
  .hint {
    max-width: 640px;
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
