<script lang="ts">
  import type { EngineHost, MonitorMode, TunePreview, TuneSettings } from "@workstation/engine-api";
  import DotSlider from "./DotSlider.svelte";
  import Menu from "./Menu.svelte";
  import Toggle from "./Toggle.svelte";
  import TuneGraph from "./TuneGraph.svelte";
  import {
    formatPercent,
    formatRetune,
    formatSemitones,
    KEYBOARD,
    listenCommands,
    monitorMode,
    monitorNext,
    pickScale,
    scaleChoices,
    scaleId,
    togglePitchClass,
    tunedVersion,
    type ApplyState,
    type Listen,
    type MonitorEvent,
    type MonitorState,
  } from "./autotune";
  import { enter } from "./motion";
  import { applyTake, runCommands } from "./recorder";
  import { errorMessage, type Session } from "./state";

  /**
   * The Auto-tune view (AGENTS.md §7): the correction controls, the live monitor (hear yourself
   * dry or tuned over the song, on headphones), and "Apply to take…", which renders a take tuned
   * into a new take, kept with the song, scored, and comparable with the original.
   */
  let { host, session, onSeek }: { host: EngineHost; session: Session; onSeek: (frame: number) => void } = $props();

  // The host is fixed for the app's lifetime.
  // svelte-ignore state_referenced_locally
  const autotune = host.autotune;

  const project = $derived(session.project);
  const tune = $derived(project.tune);
  const hasMelody = $derived(project.analysis.reference_notes.length > 0);

  const settings = (): TuneSettings => session.project.tune;

  function update(patch: Partial<TuneSettings>) {
    session.adopt({ ...session.project, tune: { ...session.project.tune, ...patch } });
    void autotune.setTune(settings()).catch((e) => console.warn(errorMessage(e)));
  }

  const modes = [
    { id: "Scale", label: "Nearest scale note" },
    { id: "FollowMelody", label: "Follow the melody" },
  ] as const;

  // ---- Live monitor ---------------------------------------------------------------------------

  let monitor = $state<MonitorState>({ kind: "off" });

  async function switchMonitor(to: MonitorMode) {
    const next = monitorNext(monitor, { kind: "request", to });
    if (!next) return;
    monitor = next;
    let event: MonitorEvent;
    try {
      if (to !== "Off") await autotune.setTune(settings());
      event = { kind: "switched", status: await autotune.setMonitor(to) };
    } catch (e) {
      event = { kind: "fail", message: errorMessage(e) };
    }
    monitor = monitorNext(monitor, event) ?? monitor;
  }

  const mode = $derived(monitorMode(monitor));

  // ---- Takes, preview and Apply ---------------------------------------------------------------

  /** The take being tuned: the newest one that isn't itself a tuned copy. */
  let picked = $state<number | null>(null);
  const take = $derived.by(() => {
    if (picked !== null && project.takes.some((t) => t.id === picked)) return picked;
    const originals = project.takes.filter((t) => t.source.kind !== "Tuned");
    return (originals.at(-1) ?? project.takes.at(-1))?.id ?? null;
  });
  const takeOptions = $derived(project.takes.map((t) => ({ id: String(t.id), label: t.name })));

  type PreviewState = { kind: "none" } | { kind: "ready"; preview: TunePreview } | { kind: "failed"; message: string };
  let preview = $state<PreviewState>({ kind: "none" });
  /** Newest preview request; older answers are dropped. */
  let asked = 0;

  $effect(() => {
    if (take === null) {
      preview = { kind: "none" };
      return;
    }
    const id = take;
    const s = $state.snapshot(tune);
    const n = ++asked;
    autotune.preview(id, s).then(
      (p) => n === asked && (preview = { kind: "ready", preview: p }),
      (e) => n === asked && (preview = { kind: "failed", message: errorMessage(e) }),
    );
  });

  let apply = $state<ApplyState>({ kind: "idle" });

  async function applyToTake() {
    if (take === null || apply.kind === "applying") return;
    apply = { kind: "applying", take };
    try {
      const added = await autotune.apply(take, settings(), $state.snapshot(session.settings));
      applyTake(session, added);
      apply = { kind: "applied", take: added.take };
    } catch (e) {
      apply = { kind: "failed", message: errorMessage(e) };
    }
  }

  /** The editable correction curve: seeds the take's pitch-editor list from these settings (each
   * note centred where auto-tune would pull it) and opens it in the Pitch view, so the curve can
   * be adjusted note by note and rendered from there (AGENTS.md §7.1). */
  let seeding = $state<{ kind: "idle" } | { kind: "seeding" } | { kind: "failed"; message: string }>({ kind: "idle" });

  async function editCurve() {
    if (take === null || seeding.kind === "seeding") return;
    const id = take;
    seeding = { kind: "seeding" };
    try {
      await host.pitchEditor.fromTune(id, settings());
      session.touch();
      session.pitchSource = { kind: "Take", id };
      session.view = "Pitch";
    } catch (e) {
      seeding = { kind: "failed", message: errorMessage(e) };
    }
  }

  // ---- A/B ------------------------------------------------------------------------------------

  const tuned = $derived(take === null ? null : tunedVersion(project, take));
  const listenOptions = $derived([
    { id: "song", label: "The song" },
    ...(take === null ? [] : [{ id: String(take), label: "Original take" }]),
    ...(tuned === null ? [] : [{ id: String(tuned), label: "Tuned take" }]),
  ]);
  let listen = $state<Listen>({ kind: "song" });

  function hear(id: string) {
    listen = id === "song" ? { kind: "song" } : { kind: "take", take: Number(id) };
    void runCommands(host, listenCommands(session.project, listen));
  }

  // Leaving the view: the monitor goes off and the mix goes back to the song.
  $effect(() => () => {
    if (monitor.kind === "on" || monitor.kind === "switching") void autotune.setMonitor("Off").catch(() => undefined);
    if (listen.kind !== "song") void runCommands(host, listenCommands(session.project, { kind: "song" }));
  });
</script>

<section class="autotune">
  <div class="controls">
    <Menu label="Key" value={scaleId(project, tune.scale)} options={scaleChoices(project)} onPick={(id) => update({ scale: pickScale(id, project, tune.scale) })} />
    {#if hasMelody}
      <Menu label="Pull to" value={tune.mode} options={modes} onPick={(m) => update({ mode: m })} />
    {/if}
  </div>

  {#if tune.scale.kind === "Custom"}
    {@const classes = tune.scale.pitch_classes}
    <div class="keyboard" role="group" aria-label="Notes to tune to" in:enter>
      {#each KEYBOARD as k (k.pc)}
        <button
          class="press key"
          class:black={k.black}
          class:on={(classes & (1 << k.pc)) !== 0}
          aria-pressed={(classes & (1 << k.pc)) !== 0}
          aria-label="Pitch class {k.pc}"
          onclick={() => update({ scale: { kind: "Custom", pitch_classes: togglePitchClass(classes, k.pc) } })}
        ></button>
      {/each}
    </div>
  {/if}

  <div class="sliders">
    <label class="slider">
      <span class="label">Retune speed</span>
      <DotSlider value={tune.retune_ms} min={0} max={400} step={5} width={200} label="Retune speed" readout={formatRetune(tune.retune_ms)} onChange={(v) => update({ retune_ms: v })} />
    </label>
    <label class="slider">
      <span class="label">Humanize</span>
      <DotSlider value={tune.humanize} min={0} max={1} step={0.01} width={200} label="Humanize" readout={formatPercent(tune.humanize)} onChange={(v) => update({ humanize: v })} />
    </label>
    <label class="slider">
      <span class="label">Formant</span>
      <DotSlider value={tune.formant_semitones} min={-5} max={5} step={0.1} width={200} label="Formant shift" readout={formatSemitones(tune.formant_semitones)} onChange={(v) => update({ formant_semitones: v })} />
    </label>
    <label class="slider">
      <span class="label">Mix</span>
      <DotSlider value={tune.mix} min={0} max={1} step={0.01} width={200} label="Dry/wet mix" readout={formatPercent(tune.mix)} onChange={(v) => update({ mix: v })} />
    </label>
  </div>

  <div class="monitor">
    <Toggle checked={mode !== "Off"} label="Hear yourself live" onToggle={(on) => void switchMonitor(on ? (mode === "Dry" ? "Dry" : "Tuned") : "Off")} />
    <Toggle checked={mode !== "Dry"} label="Tuned" onToggle={(on) => mode !== "Off" && void switchMonitor(on ? "Tuned" : "Dry")} />
    <p class="muted hint">
      {#if monitor.kind === "on"}
        {monitor.status.mode === "Tuned" ? "Tuned" : "Dry"}, about {monitor.status.latency_ms.toFixed(0)} ms behind you. Headphones only: speakers feed your voice back into the microphone.
      {:else if monitor.kind === "failed"}
        Couldn't monitor: {monitor.message}
      {:else if monitor.kind === "switching"}
        Opening the microphone
      {:else}
        Headphones only: speakers feed your voice back into the microphone.
      {/if}
    </p>
  </div>

  <div class="takes">
    {#if take === null}
      <div class="caption">
        <h2>No takes yet</h2>
        <p class="muted">Record or import a take in the Score tab to tune it. You can monitor yourself live here meanwhile.</p>
      </div>
    {:else}
      <div class="controls">
        <Menu label="Take" value={String(take)} options={takeOptions} onPick={(id) => (picked = Number(id))} />
        <Menu label="Hear" value={listen.kind === "song" ? "song" : String(listen.take)} options={listenOptions} onPick={hear} />
        <span class="spacer"></span>
        <button class="press" onclick={editCurve} disabled={seeding.kind === "seeding"} title="Open this correction in the Pitch view to adjust it note by note">Edit curve…</button>
        <button class="press primary big" onclick={applyToTake} disabled={apply.kind === "applying"}>
          {apply.kind === "applying" ? "Tuning the take" : "Apply to take…"}
        </button>
      </div>
      {#if apply.kind === "applied"}
        {@const id = apply.take}
        {@const added = project.takes.find((t) => t.id === id)}
        <p class="muted" in:enter>Added “{added?.name}”, kept with the song. Its score is in the Score tab; compare it with the original under Hear.</p>
      {:else if apply.kind === "failed"}
        <p class="muted" in:enter>Couldn't tune the take: {apply.message}</p>
      {/if}
      {#if seeding.kind === "failed"}
        <p class="muted" in:enter>Couldn't open the curve: {seeding.message}</p>
      {/if}
      {#if preview.kind === "ready"}
        <TuneGraph preview={preview.preview} onSeek={(s) => onSeek(Math.floor(s * session.project.sample_rate))} />
      {:else if preview.kind === "failed"}
        <p class="muted">{preview.message}</p>
      {/if}
    {/if}
  </div>
</section>

<style>
  .autotune {
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
  .sliders {
    display: flex;
    gap: 32px;
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
  .keyboard {
    display: flex;
    gap: 4px;
    margin-top: -16px;
  }
  .key {
    width: 28px;
    height: 44px;
    padding: 0;
    border-radius: 6px;
    background: var(--c-raised);
    box-shadow: inset 0 0 0 1px var(--c-disabled);
  }
  .key.black {
    height: 32px;
    background: var(--c-surface);
  }
  .key.on {
    background: var(--c-vocals);
    box-shadow: none;
  }
  .monitor {
    display: flex;
    align-items: center;
    gap: 32px;
    flex-wrap: wrap;
  }
  .hint {
    flex-basis: 100%;
  }
  .takes {
    display: flex;
    flex-direction: column;
    gap: 24px;
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
</style>
