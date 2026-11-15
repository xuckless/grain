<script lang="ts">
  import type { EngineHost, Latency } from "@workstation/engine-api";
  import Toggle from "./Toggle.svelte";
  import { saveCalibrateFirst } from "./prefs";
  import { monitorMode, switchMonitor } from "./autotune";
  import { services } from "./context";
  import { LISTEN_MODES } from "./listen";
  import { popover } from "./motion";
  import { errorMessage, type Session } from "./state";
  import { calibrationNext, LATENCY_NUDGE_MS, type Calibration, type CalibrationEvent } from "./takes";

  /**
   * ⚙ Settings (AGENTS.md §8.6): how takes play back, the microphone's round-trip latency and its
   * loopback calibration (§9), and live monitoring. Opened from the top bar; one place instead of
   * a copy in every view.
   */
  let { host, session, onClose, beforeCalibrate }: { host: EngineHost; session: Session; onClose: () => void; beforeCalibrate: () => void } = $props();

  const { toast } = services();
  // The host is fixed for the app's lifetime.
  // svelte-ignore state_referenced_locally
  const mic = host.microphone;

  let calibration = $state<Calibration>({ kind: "idle" });
  let latency = $state<Latency | null>(null);
  let root = $state<HTMLDivElement | undefined>();

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
    if (mic.kind !== "available" || session.mic.kind !== "free" || !calibrationEvent({ kind: "start" })) return;
    beforeCalibrate();
    session.mic = { kind: "calibrating" };
    try {
      latency = await mic.measureLatency();
      calibrationEvent({ kind: "done", latency });
    } catch (e) {
      calibrationEvent({ kind: "fail", message: errorMessage(e) });
    } finally {
      session.mic = { kind: "free" };
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

  const mode = $derived(monitorMode(session.monitor));
  const monitorTo = (to: "Off" | "Dry" | "Tuned") => void switchMonitor(host, session, to, session.project.tune);
  const busy = $derived(session.mic.kind !== "free");

  $effect(() => {
    const away = (e: PointerEvent) => {
      if (root && !root.contains(e.target as Node) && !(e.target as Element).closest?.("[data-settings-trigger]")) onClose();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("pointerdown", away);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("pointerdown", away);
      window.removeEventListener("keydown", esc);
    };
  });
</script>

<div class="panel" role="dialog" aria-label="Settings" bind:this={root} transition:popover>
  <section>
    <h3>Playback</h3>
    <div class="segments" role="radiogroup" aria-label="How takes play back">
      {#each LISTEN_MODES as m (m.id)}
        <button class="press seg" class:on={session.playback === m.id} role="radio" aria-checked={session.playback === m.id} onclick={() => (session.playback = m.id)}>{m.label}</button>
      {/each}
    </div>
    <p class="muted">The guide vocal you can sing along to is never in takes, playback or exports.</p>
  </section>

  {#if mic.kind === "available"}
    <section>
      <h3>Microphone latency</h3>
      <div class="row">
        {#if latency}
          <div class="stepper" role="group" aria-label="Microphone latency">
            <button class="press step" aria-label="{LATENCY_NUDGE_MS} ms less" disabled={busy} onclick={() => nudge(-LATENCY_NUDGE_MS)}>−</button>
            <span class="value mono">{latency.ms.toFixed(0)} ms</span>
            <button class="press step" aria-label="{LATENCY_NUDGE_MS} ms more" disabled={busy} onclick={() => nudge(LATENCY_NUDGE_MS)}>+</button>
          </div>
          <span class="muted small">{describeLatency(latency)}</span>
        {/if}
        <span class="spacer"></span>
        <button class="press raised small-btn" onclick={calibrate} disabled={busy}>{calibration.kind === "measuring" ? "Listening…" : "Calibrate"}</button>
      </div>
      <p class="muted">
        {#if calibration.kind === "measuring"}
          Playing a short chirp. Keep still and quiet.
        {:else if calibration.kind === "failed"}
          {calibration.message}
        {:else if calibration.kind === "measured"}
          Measured {calibration.latency.ms.toFixed(0)} ms. Plug your headphones back in to record.
        {:else}
          Calibrate once per microphone and output: unplug your headphones, or hold them against the microphone, so it can hear the test chirp.
        {/if}
      </p>
      <div class="row">
        <Toggle
          checked={session.calibrateFirst}
          label="Calibrate before each recording"
          onToggle={(on) => {
            session.calibrateFirst = on;
            saveCalibrateFirst(on);
          }}
        />
      </div>
      <p class="muted">A one-second test sound before Sing and Retake. If it isn't heard (headphones on), your last latency is used and recording goes ahead.</p>
    </section>

    <section>
      <h3>Monitoring</h3>
      <div class="row">
        <Toggle checked={mode !== "Off"} label="Hear yourself live" onToggle={(on) => monitorTo(on ? (mode === "Dry" ? "Dry" : "Tuned") : "Off")} />
        <Toggle checked={mode === "Tuned"} label="Tuned" disabled={mode === "Off"} onToggle={(on) => monitorTo(on ? "Tuned" : "Dry")} />
      </div>
      <p class="muted">
        {#if session.monitor.kind === "on"}
          {session.monitor.status.mode === "Tuned" ? "Tuned" : "Dry"}, about {session.monitor.status.latency_ms.toFixed(0)} ms behind you.
        {:else if session.monitor.kind === "failed"}
          Couldn't monitor: {session.monitor.message}
        {/if}
        Headphones only: speakers feed your voice back into the microphone.
      </p>
    </section>
  {:else}
    <section>
      <h3>Microphone</h3>
      <p class="muted">{mic.reason}</p>
    </section>
  {/if}
</div>

<style>
  .panel {
    position: absolute;
    right: 0;
    top: 52px;
    width: 440px;
    box-sizing: border-box;
    padding: 18px 20px;
    background: var(--c-raised);
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    gap: 20px;
    transform-origin: top right;
    z-index: 40;
    font-size: 14px;
    line-height: 21px;
  }
  h3 {
    margin: 0 0 8px;
    font-size: 13px;
    line-height: 18px;
    font-weight: 500;
    color: var(--c-muted);
  }
  p {
    margin: 8px 0 0;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px 12px;
    flex-wrap: wrap;
  }
  .small {
    font-size: 13px;
  }
  .spacer {
    flex: 1;
  }
  .stepper {
    display: flex;
    align-items: center;
    height: 36px;
    border-radius: 10px;
    background: var(--c-surface);
  }
  .step {
    width: 36px;
    height: 36px;
    border-radius: 10px;
    padding: 0;
    font-size: 17px;
  }
  .step:hover:not(:disabled) {
    background: var(--c-raised);
  }
  .value {
    min-width: 48px;
    text-align: center;
    font-size: 13px;
  }
  .small-btn {
    height: 36px;
    padding: 0 14px;
    border-radius: 10px;
    background: var(--c-surface);
  }
  .segments {
    display: flex;
    gap: 4px;
    background: var(--c-surface);
    border-radius: 10px;
    padding: 4px;
  }
  .seg {
    flex: 1 1 auto;
    padding: 0 12px;
    white-space: nowrap;
    height: 36px;
    border-radius: 8px;
    font-size: 13px;
    background: transparent;
    color: var(--c-muted);
  }
  .seg.on {
    background: var(--c-text);
    color: var(--c-ground);
  }
</style>
