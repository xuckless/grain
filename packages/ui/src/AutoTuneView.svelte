<script lang="ts">
  import type { EngineHost, TunePreview, TuneSettings } from "@workstation/engine-api";
  import BottomAction from "./BottomAction.svelte";
  import DotSlider from "./DotSlider.svelte";
  import Menu from "./Menu.svelte";
  import TakePicker from "./TakePicker.svelte";
  import Toggle from "./Toggle.svelte";
  import Toolbar, { type OverflowItem } from "./Toolbar.svelte";
  import TuneGraph from "./TuneGraph.svelte";
  import ZoomControls from "./ZoomControls.svelte";
  import { formatPercent, formatRetune, formatSemitones, KEYBOARD, monitorMode, pickScale, scaleChoices, scaleId, switchMonitor, togglePitchClass } from "./autotune";
  import { commitChain, selectedTakeId, takeById, withActive, withBypass, withUnset, type ChainJob } from "./chain";
  import { enter } from "./motion";
  import { errorMessage, type Session } from "./state";

  /**
   * The Auto-tune view (AGENTS.md §7, §8.6): the take picker, key and the live monitor in the
   * toolbar; the correction controls; the before/after preview on the shared timeline; and Apply
   * at the bottom, which sets the Auto-tune step of the take's processing chain (§8.7). The take
   * is never copied: bypass or remove the step to hear it without.
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

  // ---- Live monitor (shared with Settings) -----------------------------------------------------

  const mode = $derived(monitorMode(session.monitor));
  const monitorTo = (to: "Off" | "Dry" | "Tuned") => void switchMonitor(host, session, to, settings());

  // ---- The take, preview and Apply -------------------------------------------------------------

  const take = $derived(selectedTakeId(project, session.take));
  const chain = $derived(takeById(project, take)?.chain ?? null);
  const step = $derived(chain?.tune ?? { state: "Unset" as const });

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

  let job = $state<ChainJob>({ kind: "idle" });

  async function change(next: (c: NonNullable<typeof chain>) => NonNullable<typeof chain>) {
    if (take === null || !chain || job.kind === "rendering") return;
    job = { kind: "rendering", take, step: "tune" };
    job = await commitChain(host, session, session.project, take, next($state.snapshot(chain)), "tune");
  }

  const apply = () => change((c) => withActive(c, "tune", $state.snapshot(settings())));
  const applied = $derived(step.state !== "Unset" && JSON.stringify(step.params) === JSON.stringify(tune));

  /** The editable correction curve: seeds the take's pitch-editor list from these settings (each
   * note centred where auto-tune would pull it) and opens it in the Pitch view (AGENTS.md §7.1). */
  async function editCurve() {
    if (take === null) return;
    try {
      await host.pitchEditor.fromTune(take, settings());
      session.touch();
      session.pitchRevision += 1;
      session.view = "Pitch";
    } catch (e) {
      job = { kind: "failed", message: `Couldn't open the curve: ${errorMessage(e)}` };
    }
  }

  const overflow = $derived<OverflowItem[]>([
    { label: "Edit curve in Pitch…", run: editCurve, disabled: take === null ? "No take yet" : null },
    { label: "Remove auto-tune from this take", run: () => change((c) => withUnset(c, "tune")), disabled: step.state === "Unset" ? "Auto-tune isn't on this take" : null },
  ]);

  const hints = $derived([
    "Headphones only for live monitoring: speakers feed your voice back into the microphone.",
    ...(session.monitor.kind === "on" ? [`Monitoring ${session.monitor.status.mode === "Tuned" ? "tuned" : "dry"}, about ${session.monitor.status.latency_ms.toFixed(0)} ms behind you.`] : []),
    "Apply puts auto-tune on the take itself, after its pitch edits and before clean-up and the vocal chain. The recording underneath is kept: bypass or remove it any time, and ⌘Z undoes it.",
  ]);
</script>

<section class="autotune">
  <Toolbar {hints} {overflow}>
    <TakePicker {host} {session} />
    <Menu label="Key" value={scaleId(project, tune.scale)} options={scaleChoices(project)} onPick={(id) => update({ scale: pickScale(id, project, tune.scale) })} />
    {#if hasMelody}
      <Menu label="Pull to" value={tune.mode} options={modes} onPick={(m) => update({ mode: m })} />
    {/if}
    <Toggle checked={mode !== "Off"} label="Monitor" onToggle={(on) => monitorTo(on ? (mode === "Dry" ? "Dry" : "Tuned") : "Off")} />
    <Toggle checked={mode === "Tuned"} label="Tuned" onToggle={(on) => mode !== "Off" && monitorTo(on ? "Tuned" : "Dry")} />
    {#snippet right()}<ZoomControls {session} />{/snippet}
  </Toolbar>

  {#if session.monitor.kind === "failed"}
    <p class="muted" in:enter>Couldn't monitor: {session.monitor.message}</p>
  {/if}

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

  {#if take === null}
    <div class="caption">
      <h2>No takes yet</h2>
      <p class="muted">Sing a take in Karaoke, or import one in Score, to tune it. You can monitor yourself live meanwhile.</p>
    </div>
  {:else}
    {#if preview.kind === "ready"}
      <TuneGraph preview={preview.preview} view={session.viewport} duration={session.duration} onView={(v) => (session.viewport = v)} onSeek={(s) => onSeek(Math.floor(s * session.project.sample_rate))} />
    {:else if preview.kind === "failed"}
      <p class="muted">{preview.message}</p>
    {/if}
    {#if job.kind === "failed"}
      <p class="muted" in:enter>Couldn't change the take: {job.message}</p>
    {/if}
  {/if}

  {#if take !== null}
    <BottomAction>
      <button class="press cta" onclick={apply} disabled={job.kind === "rendering" || (applied && step.state === "Active")}>
        {job.kind === "rendering" ? "Rendering" : step.state === "Unset" ? "Apply to take" : applied && step.state === "Active" ? "Applied" : "Update take"}
      </button>
      {#snippet side()}
        {#if step.state !== "Unset"}
          <Toggle checked={step.state === "Active"} label="On" onToggle={(on) => change((c) => withBypass(c, "tune", !on))} />
        {/if}
      {/snippet}
    </BottomAction>
  {/if}
</section>

<style>
  .autotune {
    display: flex;
    flex-direction: column;
    gap: 32px;
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
