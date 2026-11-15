<script lang="ts">
  import type { MixParams, Project, Track, TrackMode, TrackSource, Transcriber, TransportSnapshot } from "@workstation/engine-api";
  import DotSlider from "./DotSlider.svelte";
  import type { Action } from "svelte/action";
  import { playhead } from "./clock";
  import { cachedLevels, dot, dotBar, drawDotWave, fit, WAVE_STEP_PX } from "./grain";
  import { detailDelay, enter, reveal, rubberBand, springBack } from "./motion";
  import { laneRows, layerOf, type MidiJob } from "./models";
  import { errorMessage, formatDb, formatPan, transportSeconds } from "./state";
  import { colour, displayRank, stemHue, trackHue } from "./theme";

  /**
   * The Mix view (design: Components → Tracks): one row per stem with mute, solo, gain and pan,
   * a dot waveform in the stem's hue, and a grey copy that wipes over it when the track is
   * inaudible (rule 17). Scrubbing any lane moves the playhead directly (rule 14) and rubber-bands
   * past either end (rule 19). "MIDI" transcribes a track (basic-pitch, or the drum classifier)
   * into a MIDI lane under it, exportable as a .mid file (AGENTS.md §4.3).
   */
  let {
    project,
    snapshot,
    transcriber,
    onSeek,
    onMix,
    onProject,
    onModels,
  }: {
    project: Project;
    snapshot: TransportSnapshot;
    transcriber: Transcriber;
    onSeek: (frame: number) => void;
    onMix: (track: number, params: MixParams) => void;
    /** The engine returned an updated project (a new MIDI layer). */
    onProject: (project: Project) => void;
    /** Opens the model manager. */
    onModels: () => void;
  } = $props();

  const MIDI_H = 40;
  /** Transcription jobs by track name. */
  let jobs = $state<Record<string, MidiJob>>({});
  const jobOf = (t: Track): MidiJob => jobs[t.name] ?? { kind: "idle" };

  async function transcribe(t: Track) {
    if (jobOf(t).kind === "transcribing") return;
    jobs[t.name] = { kind: "transcribing" };
    try {
      const out = await transcriber.transcribe(t.source);
      switch (out.kind) {
        case "Done":
          jobs[t.name] = { kind: "idle" };
          onProject(out.project);
          break;
        case "NeedsModel":
          jobs[t.name] = { kind: "needsModel", model: out.model };
          break;
      }
    } catch (e) {
      jobs[t.name] = { kind: "failed", message: errorMessage(e) };
    }
  }

  async function exportMidi(t: Track) {
    try {
      await transcriber.exportMidi(t.source);
    } catch (e) {
      jobs[t.name] = { kind: "failed", message: errorMessage(e) };
    }
  }

  const LANE_H = 52;
  const RULER_H = 28;
  const RUBBER_LIMIT_PX = 24;
  /** Fader law (design: Components → Gain): slider position 0–1 maps to −60…+6 dB, 0 = silent. */
  const DB_FLOOR = -60;
  const DB_SPAN = 66;
  const gainToFader = (g: number) => (g <= 0 ? 0 : Math.max(0, Math.min(1, (20 * Math.log10(g) - DB_FLOOR) / DB_SPAN)));
  const faderToGain = (v: number) => (v <= 0 ? 0 : 10 ** ((DB_FLOOR + DB_SPAN * v) / 20));

  type Scrub =
    | { kind: "idle" }
    | { kind: "scrubbing"; seconds: number; overshootPx: number }
    | { kind: "settling"; seconds: number; overshootPx: number; cancel: () => void };
  let scrub = $state<Scrub>({ kind: "idle" });

  let width = $state(0);
  let rulerCanvas = $state<HTMLCanvasElement | undefined>();
  let refCanvas = $state<HTMLCanvasElement | undefined>();
  /** Replays the lanes' reveal for a new song only (not on every project update). */
  const revealKey = $derived(`${project.song.kind}:${project.frames}:${project.tracks.length}`);

  const duration = $derived(project.frames / project.sample_rate);
  const anySolo = $derived(project.tracks.some((t) => t.mix.mode === "Soloed"));
  const position = $derived.by(() => {
    if (scrub.kind !== "idle") return scrub.seconds;
    return transportSeconds(snapshot);
  });
  /** Tracks in display order; `index` stays the engine's track index for mix commands. */
  const rows = $derived(project.tracks.map((track, index) => ({ track, index })).sort((a, b) => displayRank(a.track.source) - displayRank(b.track.source)));
  const columns = $derived(Math.ceil(width / WAVE_STEP_PX));

  function audible(t: Track): boolean {
    return t.mix.mode === "Soloed" || (t.mix.mode === "Normal" && !anySolo);
  }

  function toggled(mode: TrackMode, target: "Muted" | "Soloed"): TrackMode {
    return mode === target ? "Normal" : target;
  }

  /** Playhead x in a lane: the clock (extrapolated per frame), or the scrub position while
   * scrubbing; moved by transform only (AGENTS.md §10.1). */
  const headX = (seconds: number): number => {
    const s = scrub.kind === "idle" ? seconds : scrub.seconds;
    return (s / duration) * width + (scrub.kind === "idle" ? 0 : scrub.overshootPx);
  };

  // Lanes are drawn only when what they show changes (never per frame).
  $effect(() => {
    if (width <= 0 || !rulerCanvas) return;
    const c = fit(rulerCanvas, width, RULER_H);
    if (!c) return;
    const x = (s: number) => (s / duration) * width;
    const beats = project.analysis.tempo.beats;
    const barPx = beats.length > 4 ? x(beats[4]) - x(beats[0]) : width;
    const labelEvery = Math.max(1, 2 ** Math.ceil(Math.log2(56 / Math.max(1, barPx))));
    c.font = '13px "Geist Mono Variable", monospace';
    c.textBaseline = "top";
    beats.forEach((b, i) => {
      const bar = i % 4 === 0;
      dot(c, x(b), RULER_H - 5, bar ? 1.4 : 0.8, bar ? colour.text : colour.muted);
      if (bar && (i / 4) % labelEvery === 0) {
        c.fillStyle = colour.muted;
        c.fillText(String(i / 4 + 1), x(b) + 4, 2);
      }
    });
  });

  $effect(() => {
    if (width <= 0 || !refCanvas) return;
    const c = fit(refCanvas, width, LANE_H);
    const notes = project.analysis.reference_notes;
    if (!c || notes.length === 0) return;
    const x = (s: number) => (s / duration) * width;
    const lo = Math.min(...notes.map((n) => n.midi)) - 1;
    const hi = Math.max(...notes.map((n) => n.midi)) + 1;
    const y = (m: number) => LANE_H - 8 - ((m - lo) / Math.max(1, hi - lo)) * (LANE_H - 16);
    for (const n of notes) dotBar(c, x(n.start), Math.max(x(n.start), x(n.end)), y(n.midi), stemHue.Vocals, 0.9, 1, 3);
  });

  type LaneArt = { width: number; draw: (c: CanvasRenderingContext2D) => void; key: unknown[] };
  /** Draws a lane canvas once per distinct `key` (its data, size and colours), never per frame. */
  const art: Action<HTMLCanvasElement, LaneArt> = (canvas, initial) => {
    let key: unknown[] = [];
    const draw = (l: LaneArt) => {
      if (l.width <= 0 || (l.key.length === key.length && l.key.every((k, i) => k === key[i]))) return;
      key = l.key;
      const c = fit(canvas, l.width, canvas.clientHeight);
      if (c) l.draw(c);
    };
    draw(initial);
    return { update: draw };
  };

  const waveArt = (t: Track, col: string): LaneArt => {
    const levels = cachedLevels(t.peaks, columns);
    return { width, key: [levels, t.mix.gain, col, width], draw: (c) => drawDotWave(c, width, LANE_H, levels, t.mix.gain, col) };
  };

  const midiArt = (t: Track): LaneArt => {
    const layer = layerOf(project, t.source);
    return {
      width,
      key: [layer, width],
      draw: (c) => {
        if (!layer) return;
        const x = (s: number) => (s / duration) * width;
        const row = laneRows(layer);
        const hue = trackHue(t.source);
        for (const n of layer.notes) {
          const x0 = x(n.start);
          dotBar(c, x0, Math.max(x0, x(n.end)), 4 + row(n) * (MIDI_H - 8), hue, 0.6 + (n.velocity / 127) * 0.6, 1, 3);
        }
      },
    };
  };

  function scrubTo(event: PointerEvent) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const px = event.clientX - rect.left;
    const clamped = Math.min(rect.width, Math.max(0, px));
    const seconds = (clamped / rect.width) * duration;
    scrub = { kind: "scrubbing", seconds, overshootPx: rubberBand(px - clamped, RUBBER_LIMIT_PX) };
    // The last frame is not seekable; stay one frame inside the song.
    onSeek(Math.min(Math.floor(duration * snapshot.sample_rate) - 1, Math.floor(seconds * snapshot.sample_rate)));
  }

  function scrubStart(event: PointerEvent) {
    if (scrub.kind === "settling") scrub.cancel();
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    scrubTo(event);
  }

  function scrubMove(event: PointerEvent) {
    if (scrub.kind === "scrubbing") scrubTo(event);
  }

  function scrubEnd() {
    if (scrub.kind !== "scrubbing") return;
    const { seconds, overshootPx } = scrub;
    const cancel = springBack(
      overshootPx,
      (v) => {
        if (scrub.kind === "settling") scrub = { ...scrub, overshootPx: v };
      },
      () => {
        scrub = { kind: "idle" };
      },
    );
    if (scrub.kind === "scrubbing") scrub = { kind: "settling", seconds, overshootPx, cancel };
  }
</script>

{#snippet lane(content: import("svelte").Snippet, height: number, label: string)}
  <div
    class="lane"
    style:height="{height}px"
    role="slider"
    tabindex="-1"
    aria-label={label}
    aria-valuemin={0}
    aria-valuemax={Math.round(duration)}
    aria-valuenow={Math.round(position)}
    onpointerdown={scrubStart}
    onpointermove={scrubMove}
    onpointerup={scrubEnd}
    onpointercancel={scrubEnd}
  >
    {@render content()}
    <div class="playhead" use:playhead={headX} style:height="{height}px"></div>
  </div>
{/snippet}

<div class="mix">
  <div class="row ruler">
    <div class="controls"></div>
    <div class="lane-box" bind:clientWidth={width}>
      {#snippet rulerLane()}<canvas bind:this={rulerCanvas} style:height="{RULER_H}px"></canvas>{/snippet}
      {@render lane(rulerLane, RULER_H, "Playhead")}
    </div>
    <div class="readout"></div>
  </div>

  <div class="row track" in:enter={{ delay: detailDelay(0) }}>
    <div class="controls">
      <div class="name">
        <span class="swatch" style:background={stemHue.Vocals}></span>
        <span class="title">Reference vocal</span>
      </div>
      <span class="muted small">{project.analysis.reference_source === "VocalStem" ? "Melody from the vocal stem" : "Melody from the full mix (no stems): expect errors"}</span>
    </div>
    <div class="lane-box">
      {#snippet refLane()}<canvas bind:this={refCanvas} use:reveal={revealKey} style:height="{LANE_H}px"></canvas>{/snippet}
      {@render lane(refLane, LANE_H, "Reference vocal, playhead")}
    </div>
    <div class="readout mono muted">{project.analysis.reference_notes.length} notes</div>
  </div>

  {#each rows as { track: t, index: i }, row (t.name)}
    <div class="row track" in:enter={{ delay: detailDelay(row + 1) }}>
      <div class="controls">
        <div class="name">
          <span class="swatch" style:background={trackHue(t.source)}></span>
          <span class="title">{t.name}</span>
        </div>
        <div class="buttons">
          <button
            class="press ms"
            class:on={t.mix.mode === "Muted"}
            aria-pressed={t.mix.mode === "Muted"}
            aria-label="Mute {t.name}"
            onclick={() => onMix(i, { ...t.mix, mode: toggled(t.mix.mode, "Muted") })}>M</button
          >
          <button
            class="press ms"
            class:on={t.mix.mode === "Soloed"}
            aria-pressed={t.mix.mode === "Soloed"}
            aria-label="Solo {t.name}"
            onclick={() => onMix(i, { ...t.mix, mode: toggled(t.mix.mode, "Soloed") })}>S</button
          >
        </div>
        <DotSlider
          value={gainToFader(t.mix.gain)}
          min={0}
          max={1}
          step={0.005}
          width={150}
          label="{t.name} gain"
          readout={formatDb(t.mix.gain)}
          onChange={(v) => onMix(i, { ...t.mix, gain: faderToGain(v) })}
        />
        <DotSlider
          value={t.mix.pan}
          min={-1}
          max={1}
          step={0.02}
          width={88}
          label="{t.name} pan"
          readout={formatPan(t.mix.pan)}
          onChange={(v) => onMix(i, { ...t.mix, pan: v })}
        />
        <span class="mono muted pan">{formatPan(t.mix.pan)}</span>
        {#if !layerOf(project, t.source)}
          <button class="press quiet midi" disabled={jobOf(t).kind === "transcribing"} title="Transcribe {t.name} to MIDI" onclick={() => transcribe(t)}>
            {jobOf(t).kind === "transcribing" ? "Transcribing" : "MIDI"}
          </button>
        {/if}
      </div>
      <div class="lane-box">
        {#snippet trackLane()}
          <canvas use:reveal={revealKey} use:art={waveArt(t, trackHue(t.source))} style:height="{LANE_H}px"></canvas>
          <canvas class="mask" class:on={!audible(t)} use:art={waveArt(t, colour.disabled)} style:height="{LANE_H}px"></canvas>
        {/snippet}
        {@render lane(trackLane, LANE_H, `${t.name} lane, playhead`)}
      </div>
      <div class="readout mono muted">{formatDb(t.mix.gain)}</div>
    </div>
    {@const job = jobOf(t)}
    {@const layer = layerOf(project, t.source)}
    {#if layer}
      <div class="row midi-row" in:enter>
        <div class="controls">
          <span class="name muted small">{t.name} MIDI</span>
          <span class="muted small">{layer.notes.length} notes · {layer.method === "DrumClassifier" ? "kick, snare, hat" : "basic-pitch"}</span>
          <span class="spacer"></span>
          <button class="press quiet" disabled={job.kind === "transcribing"} onclick={() => transcribe(t)}>{job.kind === "transcribing" ? "Transcribing" : "Redo"}</button>
          <button class="press quiet" onclick={() => exportMidi(t)}>Export .mid</button>
        </div>
        <div class="lane-box">
          {#snippet midiLane()}<canvas use:reveal={`${t.name}:${layer.notes.length}`} use:art={midiArt(t)} style:height="{MIDI_H}px"></canvas>{/snippet}
          {@render lane(midiLane, MIDI_H, `${t.name} MIDI lane, playhead`)}
        </div>
        <div class="readout"></div>
      </div>
    {/if}
    {#if job.kind === "needsModel"}
      <div class="row notice" in:enter>
        <span class="muted small">Transcribing {t.name} needs the {job.model} model.</span>
        <button class="press" onclick={onModels}>Open Models</button>
        <button class="press quiet" onclick={() => transcribe(t)}>Try again</button>
      </div>
    {:else if job.kind === "failed"}
      <div class="row notice" in:enter>
        <span class="muted small">Couldn't transcribe {t.name}: {job.message}</span>
      </div>
    {/if}
  {/each}
</div>

<style>
  .mix {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 20px;
    box-sizing: border-box;
    padding: 0 12px 0 24px;
  }
  .track {
    height: 68px;
    background: var(--c-surface);
    border-radius: 12px;
  }
  .controls {
    width: 640px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .ruler .controls {
    height: 1px;
  }
  .midi-row {
    height: 48px;
    margin-top: -6px;
  }
  .midi-row .controls {
    gap: 12px;
    padding-left: 22px;
    box-sizing: border-box;
  }
  .notice {
    gap: 12px;
    padding-left: 46px;
  }
  .spacer {
    flex: 1;
  }
  .small {
    font-size: 13px;
    line-height: 18px;
  }
  .midi {
    flex: none;
  }
  .name {
    width: 136px;
    display: flex;
    align-items: center;
    gap: 12px;
    flex: none;
  }
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex: none;
  }
  .title {
    font-size: 15px;
    line-height: 22px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .buttons {
    display: flex;
    gap: 4px;
    flex: none;
  }
  .ms {
    width: 44px;
    height: 44px;
    border-radius: 8px;
    background: var(--c-raised);
    font-size: 13px;
    font-weight: 500;
    transition:
      background-color var(--dur-colour) var(--ease-out),
      color var(--dur-colour) var(--ease-out),
      transform var(--dur-press) var(--ease-out);
  }
  .ms.on {
    background: var(--c-text);
    color: var(--c-ground);
  }
  .pan {
    width: 32px;
  }
  .lane-box {
    flex: 1;
    min-width: 0;
  }
  .lane {
    position: relative;
    cursor: pointer;
    touch-action: none;
  }
  .lane:focus {
    outline: none;
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    display: block;
  }
  .mask {
    background: var(--c-surface);
    clip-path: inset(0 100% 0 0);
    transition: clip-path var(--dur-wipe) var(--ease-io);
  }
  .mask.on {
    clip-path: inset(0 0 0 0);
  }
  .readout {
    width: 72px;
    flex: none;
    text-align: right;
  }
</style>
