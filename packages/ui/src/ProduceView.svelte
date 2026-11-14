<script lang="ts">
  import type { ChainPick, ChainPreset, ChainReport, ChainSettings, EditSource, EngineHost, GeneratorState, Instrument, Interval, MixParams, TakeAdded } from "@workstation/engine-api";
  import DotSlider from "./DotSlider.svelte";
  import Menu from "./Menu.svelte";
  import Toggle from "./Toggle.svelte";
  import { dotFill } from "./grain";
  import { enter } from "./motion";
  import { sourceFromId, sourceId, sourceOptions } from "./pitch";
  import {
    CHAIN_CONTROLS,
    INSTRUMENTS,
    INTERVALS,
    PRESETS,
    TASKS,
    canGenerate,
    describeOutput,
    generationTask,
    generatorLabel,
    instrumentLabel,
    intervalLabel,
    jobNext,
    meterRows,
    presetLabel,
    produceSource,
    producedTakes,
    randomSeed,
    type Job,
    type JobEvent,
    type TaskKind,
  } from "./produce";
  import { applyTake } from "./recorder";
  import { errorMessage, formatPan, formatTime, type Session } from "./state";
  import { takeTrack } from "./takes";
  import { colour } from "./theme";

  /**
   * The Produce view (AGENTS.md §8): clean up a take (DeepFilterNet), run it through the vocal
   * chain (offline, or live on the monitor), sing harmonies from the singer's own voice, export
   * the melody as MIDI, and generate backing with ACE-Step. Every result is a new take or track
   * that keeps how it was made (model, prompt, seed, settings); the source is never changed.
   */
  let {
    host,
    session,
    onMix,
    onModels,
  }: {
    host: EngineHost;
    session: Session;
    onMix: (track: number, params: MixParams) => void;
    onModels: () => void;
  } = $props();

  // The host is fixed for the app's lifetime.
  // svelte-ignore state_referenced_locally
  const producer = host.producer;
  const generator = producer.generator;

  const project = $derived(session.project);
  const duration = $derived(project.frames / project.sample_rate);
  const score = () => $state.snapshot(session.settings);

  // ---- Source ---------------------------------------------------------------------------------

  let picked = $state<EditSource | null>(null);
  const source = $derived.by<EditSource | null>(() => {
    const p = picked;
    if (p && (p.kind === "VocalStem" || project.takes.some((t) => t.id === p.id))) return p;
    return produceSource(project);
  });
  const sources = $derived(sourceOptions(project));

  /** Shows a new take or track; sung takes (cleaned, chained) are also scored in the Score tab. */
  function adopt(added: TakeAdded, sung: boolean) {
    if (sung) applyTake(session, added);
    else {
      session.project = added.project;
      if (session.save.kind === "clean") session.save = { kind: "dirty" };
    }
  }

  /** Runs one job: `work` resolves with what to tell the job. */
  async function run(job: Job, set: (j: Job) => void, work: () => Promise<JobEvent>) {
    const started = jobNext(job, { kind: "start" });
    if (!started) return;
    // Results don't become the source: the view stays on what was being worked on.
    if (source) picked = $state.snapshot(source) as EditSource;
    set(started);
    let event: JobEvent;
    try {
      event = await work();
    } catch (e) {
      event = { kind: "fail", message: errorMessage(e) };
    }
    set(jobNext(started, event) ?? started);
  }

  // ---- Clean up -------------------------------------------------------------------------------

  let clean = $state<Job>({ kind: "idle" });

  function cleanUp() {
    const from = source;
    const d = producer.denoise;
    if (!from || d.kind === "unavailable") return;
    void run(clean, (j) => (clean = j), async () => {
      const out = await d.run(from, score());
      if (out.kind === "NeedsModel") return { kind: "needsModel", model: out.model };
      adopt(out.added, true);
      return { kind: "added", take: out.added.take };
    });
  }

  // ---- Vocal chain ----------------------------------------------------------------------------

  let preset = $state<ChainPreset>("Auto");
  type ChainState = { kind: "measuring" } | { kind: "ready"; pick: ChainPick; settings: ChainSettings } | { kind: "failed"; message: string };
  let chain = $state<ChainState>({ kind: "measuring" });
  let report = $state<ChainReport | null>(null);
  let asked = 0;

  /** The source as a stable id, so adding a take doesn't re-measure. */
  const sid = $derived(source === null ? null : sourceId(source));

  // A new source or preset: measure and start from the preset's settings.
  $effect(() => {
    const id = sid;
    const p = preset;
    if (id === null) return;
    const n = ++asked;
    chain = { kind: "measuring" };
    report = null;
    producer.chainPick(sourceFromId(id), p).then(
      (pick) => n === asked && (chain = { kind: "ready", pick, settings: pick.settings }),
      (e) => n === asked && (chain = { kind: "failed", message: errorMessage(e) }),
    );
  });

  function tweak(key: keyof ChainSettings, value: number) {
    if (chain.kind !== "ready") return;
    chain = { ...chain, settings: { ...chain.settings, [key]: value } };
    if (live.kind === "on") void sendChain(chain.settings);
  }

  let chained = $state<Job>({ kind: "idle" });

  function applyChain() {
    const from = source;
    if (!from || chain.kind !== "ready") return;
    const settings = $state.snapshot(chain.settings) as ChainSettings;
    void run(chained, (j) => (chained = j), async () => {
      const out = await producer.chainApply(from, preset, settings, score());
      adopt(out.added, true);
      report = out.report;
      return { kind: "added", take: out.added.take };
    });
  }

  const meters = $derived(chain.kind === "ready" ? meterRows(report?.before ?? chain.pick.measures, report?.after ?? null) : []);

  /** The chain on the live monitor (the voice heard dry, then through the chain). */
  type Live = { kind: "off" } | { kind: "switching" } | { kind: "on" } | { kind: "failed"; message: string };
  let live = $state<Live>({ kind: "off" });

  async function sendChain(settings: ChainSettings | null) {
    await host.send({ kind: "Chain", settings: settings ? ($state.snapshot(settings) as ChainSettings) : null });
  }

  async function setLive(on: boolean) {
    if (live.kind === "switching" || chain.kind !== "ready") return;
    live = { kind: "switching" };
    try {
      if (on) {
        await sendChain(chain.settings);
        await host.autotune.setMonitor("Dry");
        live = { kind: "on" };
      } else {
        await host.autotune.setMonitor("Off");
        await sendChain(null);
        live = { kind: "off" };
      }
    } catch (e) {
      await sendChain(null).catch(() => undefined);
      live = { kind: "failed", message: errorMessage(e) };
    }
  }

  // Leaving the view turns the live chain and the monitor off.
  $effect(() => () => {
    if (live.kind === "on" || live.kind === "switching") {
      void host.autotune.setMonitor("Off").catch(() => undefined);
      void sendChain(null).catch(() => undefined);
    }
  });

  // ---- Harmony and MIDI -----------------------------------------------------------------------

  let harmony = $state<Job>({ kind: "idle" });
  let lastInterval = $state<Interval>("ThirdAbove");

  function sing(interval: Interval) {
    const from = source;
    if (!from) return;
    lastInterval = interval;
    void run(harmony, (j) => (harmony = j), async () => {
      const added = await producer.harmony(from, interval, score());
      adopt(added, false);
      return { kind: "added", take: added.take };
    });
  }

  let exportError = $state<string | null>(null);

  async function exportMidi(interval: Interval | null) {
    if (!source) return;
    exportError = null;
    try {
      await producer.exportMelody($state.snapshot(source) as EditSource, interval);
    } catch (e) {
      exportError = errorMessage(e);
    }
  }

  // ---- Generation -----------------------------------------------------------------------------

  let genState = $state<GeneratorState>({ kind: "NotInstalled" });
  if (generator.kind === "available") void generator.state().then((s) => (genState = s));

  let task = $state<TaskKind>("Backing");
  let instrument = $state<Instrument>("Drums");
  let prompt = $state("");
  let seed = $state(randomSeed());
  let repaintStart = $state(0);
  // The first 30 s of the song (the song doesn't change while the view is open).
  // svelte-ignore state_referenced_locally
  let repaintEnd = $state(Math.min(30, session.project.frames / session.project.sample_rate));
  let generation = $state<Job>({ kind: "idle" });

  function generate() {
    const from = source ?? { kind: "VocalStem" };
    if (generator.kind !== "available" || !canGenerate(genState)) return;
    const t = generationTask(task, $state.snapshot(from) as EditSource, instrument, repaintStart, repaintEnd);
    void run(generation, (j) => (generation = j), async () => {
      const added = await generator.generate(t, prompt, seed, score(), (s) => (genState = s));
      adopt(added, false);
      seed = randomSeed();
      return { kind: "added", take: added.take };
    });
  }

  // ---- Outputs --------------------------------------------------------------------------------

  const outputs = $derived(producedTakes(project));

  function mix(id: number, params: MixParams) {
    onMix(takeTrack(project, id), { mode: params.mode, gain: params.gain, pan: params.pan });
  }

  const nameOf = (id: number) => project.takes.find((t) => t.id === id)?.name ?? "";
</script>

{#snippet outcome(job: Job, verb: string)}
  {#if job.kind === "done"}
    <p class="muted" in:enter>Added “{nameOf(job.take)}”. The source is unchanged.</p>
  {:else if job.kind === "failed"}
    <p class="muted" in:enter>Couldn't {verb}: {job.message}</p>
  {:else if job.kind === "needsModel"}
    <p class="muted" in:enter>
      This needs the <span class="mono">{job.model}</span> model.
      <button class="press quiet inline" onclick={onModels}>Open Models</button>
    </p>
  {/if}
{/snippet}

<section class="produce">
  {#if source === null}
    <div class="caption">
      <h2>Nothing to produce yet</h2>
      <p class="muted">Record or import a take in the Score tab, or import a song with a vocal, to clean it up, shape it and add harmonies or backing.</p>
    </div>
  {:else}
    <div class="controls">
      <Menu label="Source" value={sid ?? ""} options={sources} onPick={(id) => (picked = sourceFromId(id))} />
      <p class="muted">Everything here makes a new take or track; the source is never changed.</p>
    </div>

    <div class="card">
      <div class="head">
        <h3>Clean up</h3>
        <span class="muted small">DeepFilterNet 3 noise suppression</span>
        <span class="spacer"></span>
        {#if producer.denoise.kind === "available"}
          <button class="press primary big" onclick={cleanUp} disabled={clean.kind === "running"}>{clean.kind === "running" ? "Cleaning up" : "Clean up take"}</button>
        {/if}
      </div>
      {#if producer.denoise.kind === "unavailable"}
        <p class="muted">{producer.denoise.reason}</p>
      {:else}
        <p class="muted">Removes room noise, hum and hiss under the voice (held to −30 dB, so breaths stay natural).</p>
      {/if}
      {@render outcome(clean, "clean it up")}
    </div>

    <div class="card">
      <div class="head">
        <h3>Vocal chain</h3>
        <span class="muted small">High-pass · de-esser · compressor · EQ · saturation · reverb and delay</span>
      </div>
      <div class="controls">
        <Menu label="Preset" value={preset} options={PRESETS.map((p) => ({ id: p, label: p === "Auto" ? "Auto (from this take)" : presetLabel(p) }))} onPick={(p) => (preset = p)} />
        <Toggle checked={live.kind === "on"} label="Hear it live" onToggle={(on) => void setLive(on)} />
        <span class="spacer"></span>
        <button class="press primary big" onclick={applyChain} disabled={chain.kind !== "ready" || chained.kind === "running"}>{chained.kind === "running" ? "Rendering" : "Apply to take"}</button>
      </div>
      {#if live.kind === "on"}
        <p class="muted small">Live on your voice over the song. Headphones only: speakers feed back.</p>
      {:else if live.kind === "failed"}
        <p class="muted small">Couldn't monitor: {live.message}</p>
      {/if}
      {#if chain.kind === "measuring"}
        <p class="muted">Measuring the take</p>
      {:else if chain.kind === "failed"}
        <p class="muted">{chain.message}</p>
      {:else}
        {@const s = chain.settings}
        <div class="sliders" in:enter>
          {#each CHAIN_CONTROLS as c (c.key)}
            <label class="slider">
              <span class="label">{c.label}</span>
              <DotSlider value={s[c.key]} min={c.min} max={c.max} step={c.step} width={160} label={c.label} readout={c.format(s[c.key])} onChange={(v) => tweak(c.key, v)} />
            </label>
          {/each}
        </div>
        <div class="meters" aria-label="Before and after">
          <span></span><span class="label">Before</span><span class="label">{report ? "After" : ""}</span>
          {#each meters as m (m.label)}
            <span class="label">{m.label}</span>
            <span class="meter"><canvas use:dotFill={{ fraction: m.beforeFill, lit: colour.muted }}></canvas><span class="mono small">{m.before}</span></span>
            <span class="meter">
              {#if report}<canvas use:dotFill={{ fraction: m.afterFill, lit: colour.text }}></canvas><span class="mono small">{m.after}</span>{/if}
            </span>
          {/each}
        </div>
      {/if}
      {@render outcome(chained, "render the chain")}
    </div>

    <div class="card">
      <div class="head">
        <h3>Harmony</h3>
        <span class="muted small">Diatonic, in {project.analysis.keys[0]?.label ?? "C major"}, sung by the same voice</span>
      </div>
      <div class="controls">
        {#each INTERVALS as i (i)}
          <button class="press raised big" onclick={() => sing(i)} disabled={harmony.kind === "running"}>{intervalLabel(i)}</button>
        {/each}
        <span class="spacer"></span>
        <button class="press quiet" onclick={() => exportMidi(null)}>Melody MIDI</button>
        <button class="press quiet" onclick={() => exportMidi(lastInterval)}>{intervalLabel(lastInterval)} MIDI</button>
      </div>
      {#if harmony.kind === "running"}<p class="muted">Singing the harmony</p>{/if}
      {@render outcome(harmony, "sing the harmony")}
      {#if exportError}<p class="muted">Couldn't export: {exportError}</p>{/if}
    </div>

    <div class="card">
      <div class="head">
        <h3>Generate</h3>
        <span class="muted small">ACE-Step 1.5 · MIT</span>
        <span class="spacer"></span>
        {#if generator.kind === "available"}<span class="mono small" class:muted={genState.kind !== "Ready"}>{generatorLabel(genState)}</span>{/if}
      </div>
      {#if generator.kind === "unavailable"}
        <p class="muted">{generator.reason}</p>
      {:else}
        <p class="muted small">{generator.expectations}</p>
        {#if genState.kind === "NotInstalled" || (genState.kind === "Failed" && !genState.installed)}
          <p class="muted">
            Not installed yet.
            <button class="press quiet inline" onclick={onModels}>Install it from Models</button>
          </p>
        {:else}
          <div class="controls">
            <Menu label="Make" value={task} options={TASKS} onPick={(t) => (task = t)} />
            {#if task === "Lego"}
              <Menu label="Instrument" value={instrument} options={INSTRUMENTS.map((i) => ({ id: i, label: instrumentLabel(i) }))} onPick={(i) => (instrument = i)} />
            {/if}
          </div>
          {#if task === "Repaint"}
            <div class="sliders">
              <label class="slider">
                <span class="label">From</span>
                <DotSlider value={repaintStart} min={0} max={duration} step={0.5} width={200} label="Repaint from" readout={formatTime(repaintStart, 0)} onChange={(v) => (repaintStart = v)} />
              </label>
              <label class="slider">
                <span class="label">To</span>
                <DotSlider value={repaintEnd} min={0} max={duration} step={0.5} width={200} label="Repaint to" readout={formatTime(repaintEnd, 0)} onChange={(v) => (repaintEnd = v)} />
              </label>
            </div>
          {/if}
          <div class="controls">
            <input class="prompt" type="text" placeholder="Style, e.g. warm lo-fi keys, brushed drums" bind:value={prompt} aria-label="Style prompt" />
            <label class="seed mono small">seed <input type="number" min="0" bind:value={seed} aria-label="Seed" /></label>
            <button class="press quiet" onclick={() => (seed = randomSeed())}>New seed</button>
            <button class="press primary big" onclick={generate} disabled={!canGenerate(genState) || generation.kind === "running"}>{generation.kind === "running" ? "Generating" : "Generate"}</button>
          </div>
          <p class="muted small">Conditioned on {project.analysis.tempo.bpm.toFixed(0)} BPM{project.analysis.keys[0] ? `, ${project.analysis.keys[0].label}` : ""} and {task === "Repaint" ? "the song's mix" : "the source's vocal"}. Stereo results are kept as one mono track.</p>
        {/if}
        {@render outcome(generation, "generate")}
        {#if genState.kind === "Failed"}<p class="muted small">{genState.message}</p>{/if}
      {/if}
    </div>
  {/if}

  {#if outputs.length > 0}
    <div class="card">
      <div class="head"><h3>Made here</h3></div>
      <ul class="outputs">
        {#each outputs as t (t.id)}
          <li in:enter>
            <div class="what">
              <span>{t.name}</span>
              <span class="muted small">{describeOutput(t)}</span>
            </div>
            <Toggle checked={t.mix.mode !== "Muted"} label="Play with the song" onToggle={(on) => mix(t.id, { ...t.mix, mode: on ? "Normal" : "Muted" })} />
            <DotSlider value={t.mix.gain} min={0} max={1.5} step={0.01} width={120} label="Level" readout={`${Math.round(t.mix.gain * 100)}%`} onChange={(v) => mix(t.id, { ...t.mix, gain: v })} />
            <DotSlider value={t.mix.pan} min={-1} max={1} step={0.01} width={100} label="Pan" readout={formatPan(t.mix.pan)} onChange={(v) => mix(t.id, { ...t.mix, pan: v })} />
          </li>
        {/each}
      </ul>
    </div>
  {/if}
</section>

<style>
  .produce {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .card {
    background: var(--c-surface);
    border-radius: 16px;
    padding: 20px 24px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 12px;
    flex-wrap: wrap;
  }
  h2,
  h3 {
    margin: 0;
    font-weight: 500;
  }
  h2 {
    font-size: 20px;
    line-height: 28px;
  }
  h3 {
    font-size: 16px;
    line-height: 24px;
  }
  p {
    margin: 0;
  }
  .caption {
    max-width: 560px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .controls {
    display: flex;
    align-items: center;
    gap: 16px 24px;
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
  }
  .small {
    font-size: 13px;
    line-height: 18px;
  }
  .inline {
    height: auto;
    padding: 0 4px;
    text-decoration: underline;
  }
  .sliders {
    display: flex;
    gap: 20px 28px;
    flex-wrap: wrap;
  }
  .slider {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .label {
    font-size: 13px;
    line-height: 18px;
    font-weight: 500;
    color: var(--c-muted);
  }
  .meters {
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr) minmax(0, 1fr);
    gap: 8px 16px;
    align-items: center;
    max-width: 640px;
  }
  .meter {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .meter canvas {
    width: 100%;
    height: 8px;
    display: block;
  }
  .prompt {
    flex: 1;
    min-width: 220px;
    height: 44px;
    border-radius: 12px;
    border: none;
    padding: 0 14px;
    background: var(--c-raised);
    color: var(--c-text);
    font: inherit;
  }
  .seed {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--c-muted);
  }
  .seed input {
    width: 110px;
    height: 36px;
    border-radius: 10px;
    border: none;
    padding: 0 10px;
    background: var(--c-raised);
    color: var(--c-text);
    font: inherit;
  }
  .outputs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .outputs li {
    display: flex;
    align-items: center;
    gap: 16px 24px;
    flex-wrap: wrap;
  }
  .what {
    display: flex;
    flex-direction: column;
    min-width: 200px;
    flex: 1;
  }
</style>
