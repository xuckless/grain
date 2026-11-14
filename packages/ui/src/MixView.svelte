<script lang="ts">
  import type { MixParams, Project, Track, TrackMode, TransportSnapshot } from "@workstation/engine-api";
  import DotSlider from "./DotSlider.svelte";
  import { columnLevels, dot, dotBar, drawDotWave, fit, WAVE_STEP_PX } from "./grain";
  import { detailDelay, enter, reveal, rubberBand, springBack } from "./motion";
  import { formatDb, formatPan } from "./state";
  import { colour, displayRank, stemHue, trackHue } from "./theme";

  /**
   * The Mix view (design: Components → Tracks): one row per stem with mute, solo, gain and pan,
   * a dot waveform in the stem's hue, and a grey copy that wipes over it when the track is
   * inaudible (rule 17). Scrubbing any lane moves the playhead directly (rule 14) and rubber-bands
   * past either end (rule 19).
   */
  let {
    project,
    snapshot,
    onSeek,
    onMix,
  }: {
    project: Project;
    snapshot: TransportSnapshot;
    onSeek: (frame: number) => void;
    onMix: (track: number, params: MixParams) => void;
  } = $props();

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
  const live: (HTMLCanvasElement | undefined)[] = $state([]);
  const grey: (HTMLCanvasElement | undefined)[] = $state([]);

  const duration = $derived(project.frames / project.sample_rate);
  const playing = $derived(snapshot.transport.kind === "Playing");
  const anySolo = $derived(project.tracks.some((t) => t.mix.mode === "Soloed"));
  const playhead = $derived.by(() => {
    if (scrub.kind !== "idle") return scrub.seconds;
    const t = snapshot.transport;
    return t.kind === "Stopped" ? 0 : t.frame / snapshot.sample_rate;
  });
  const headX = $derived((playhead / duration) * width + (scrub.kind === "idle" ? 0 : scrub.overshootPx));
  /** Tracks in display order; `index` stays the engine's track index for mix commands. */
  const rows = $derived(project.tracks.map((track, index) => ({ track, index })).sort((a, b) => displayRank(a.track.source) - displayRank(b.track.source)));
  const levels = $derived(project.tracks.map((t) => columnLevels(t.peaks, Math.ceil(width / WAVE_STEP_PX))));

  function audible(t: Track): boolean {
    return t.mix.mode === "Soloed" || (t.mix.mode === "Normal" && !anySolo);
  }

  function toggled(mode: TrackMode, target: "Muted" | "Soloed"): TrackMode {
    return mode === target ? "Normal" : target;
  }

  function drawPlayhead(c: CanvasRenderingContext2D, h: number) {
    c.fillStyle = colour.text;
    c.fillRect(Math.round(headX), 0, 2, h);
  }

  // Redraw lanes whenever what they show changes (the playhead moves every frame while playing).
  $effect(() => {
    if (width <= 0) return;
    const x = (s: number) => (s / duration) * width;

    if (rulerCanvas) {
      const c = fit(rulerCanvas, width, RULER_H);
      if (c) {
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
        drawPlayhead(c, RULER_H);
      }
    }

    if (refCanvas) {
      const c = fit(refCanvas, width, LANE_H);
      const notes = project.analysis.reference_notes;
      if (c && notes.length > 0) {
        const lo = Math.min(...notes.map((n) => n.midi)) - 1;
        const hi = Math.max(...notes.map((n) => n.midi)) + 1;
        const y = (m: number) => LANE_H - 8 - ((m - lo) / Math.max(1, hi - lo)) * (LANE_H - 16);
        for (const n of notes) dotBar(c, x(n.start), Math.max(x(n.start), x(n.end)), y(n.midi), stemHue.Vocals, 0.9, 1, 3);
      }
      if (c) drawPlayhead(c, LANE_H);
    }

    project.tracks.forEach((t, i) => {
      const head = playing ? headX : null;
      for (const [canvas, col] of [
        [live[i], trackHue(t.source)],
        [grey[i], colour.disabled],
      ] as const) {
        if (!canvas) continue;
        const c = fit(canvas, width, LANE_H);
        if (!c) continue;
        drawDotWave(c, width, LANE_H, levels[i], t.mix.gain, col, head);
        drawPlayhead(c, LANE_H);
      }
    });
  });

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
    aria-valuenow={Math.round(playhead)}
    onpointerdown={scrubStart}
    onpointermove={scrubMove}
    onpointerup={scrubEnd}
    onpointercancel={scrubEnd}
  >
    {@render content()}
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
      {#snippet refLane()}<canvas bind:this={refCanvas} use:reveal={project} style:height="{LANE_H}px"></canvas>{/snippet}
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
      </div>
      <div class="lane-box">
        {#snippet trackLane()}
          <canvas bind:this={live[i]} use:reveal={project} style:height="{LANE_H}px"></canvas>
          <canvas class="mask" class:on={!audible(t)} bind:this={grey[i]} style:height="{LANE_H}px"></canvas>
        {/snippet}
        {@render lane(trackLane, LANE_H, `${t.name} lane, playhead`)}
      </div>
      <div class="readout mono muted">{formatDb(t.mix.gain)}</div>
    </div>
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
    width: 560px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .ruler .controls {
    height: 1px;
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
  .small {
    font-size: 13px;
    line-height: 18px;
  }
</style>
