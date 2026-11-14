<script lang="ts">
  import type { EngineHost, Latency } from "@workstation/engine-api";
  import Menu from "./Menu.svelte";
  import { services } from "./context";
  import { effectiveBacking } from "./recorder";
  import { errorMessage, type Session } from "./state";
  import { backingNote, backingOptions, calibrationNext, defaultBacking, LATENCY_NUDGE_MS, type Calibration, type CalibrationEvent } from "./takes";

  /**
   * What a singer sets before recording, shared by Score and Karaoke: the backing they hear
   * (kept on the session) and the round-trip latency removed from takes, with the loopback
   * calibration (AGENTS.md §9). `busy`: a take is recording or being scored elsewhere;
   * `calibrating` is bound so the parent can stay idle while the chirp plays.
   */
  let {
    host,
    session,
    busy,
    calibrating = $bindable(false),
    beforeCalibrate,
  }: {
    host: EngineHost;
    session: Session;
    busy: boolean;
    calibrating?: boolean;
    beforeCalibrate: () => Promise<void>;
  } = $props();

  const { toast } = services();
  // The host is fixed for the app's lifetime.
  // svelte-ignore state_referenced_locally
  const mic = host.microphone;

  const project = $derived(session.project);
  const backings = $derived(backingOptions(project));
  const backing = $derived(effectiveBacking(session));
  const note = $derived(backing === defaultBacking(project.analysis.accompaniment) ? backingNote(project.analysis.accompaniment) : null);

  let calibration = $state<Calibration>({ kind: "idle" });
  let latency = $state<Latency | null>(null);
  $effect(() => {
    calibrating = calibration.kind === "measuring";
  });
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
    if (mic.kind !== "available" || busy || !calibrationEvent({ kind: "start" })) return;
    await beforeCalibrate();
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

  const disabled = $derived(busy || calibration.kind === "measuring");
</script>

{#if mic.kind === "available"}
  <div class="setup">
    <div class="row">
      <Menu label="Backing" value={backing} options={backings.filter((b) => b.available)} onPick={(b) => (session.backing = b)} />
      {#if note}<span class="muted">{note}</span>{/if}
    </div>
    <div class="row">
      <span class="label">Latency</span>
      {#if latency}
        <button class="press icon quiet" aria-label="{LATENCY_NUDGE_MS} ms less" {disabled} onclick={() => nudge(-LATENCY_NUDGE_MS)}>−</button>
        <span class="value-sm">{latency.ms.toFixed(0)} ms</span>
        <button class="press icon quiet" aria-label="{LATENCY_NUDGE_MS} ms more" {disabled} onclick={() => nudge(LATENCY_NUDGE_MS)}>+</button>
        <span class="muted">{describeLatency(latency)}</span>
      {/if}
      <button class="press raised calibrate" onclick={calibrate} {disabled}>
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
        Calibrate once per microphone and output: unplug your headphones, or hold them against the microphone, so it can hear
        the test chirp.
      {/if}
    </p>
  </div>
{/if}

<style>
  .setup {
    display: flex;
    flex-direction: column;
    gap: 12px;
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
    margin: 0;
  }
  .label {
    font-size: 13px;
    line-height: 18px;
    font-weight: 500;
    color: var(--c-muted);
  }
</style>
