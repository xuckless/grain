<script lang="ts">
  import type { MixParams, Project, Track, TrackMode, TrackSource, Transcriber, TransportSnapshot } from "@workstation/engine-api";
  import Icon, { type IconName } from "./Icon.svelte";
  import Knob from "./Knob.svelte";
  import type { Action } from "svelte/action";
  import { gainToKnob, knobToGain, knobToPan, panToKnob } from "./knob";
  import { timeToX, viewportGestures, xToTime, type View } from "./viewport";
  import { playhead } from "./clock";
  import { cachedLevels, dot, dotBar, drawDotWave, fit, WAVE_STEP_PX } from "./grain";
  import { detailDelay, enter, reveal, rubberBand, springBack } from "./motion";
  import { laneRows, layerOf, type MidiJob } from "./models";
  import { errorMessage, formatDb, formatPan, transportSeconds } from "./state";
  import { colour, displayRank, stemHue, trackHue } from "./theme";

  /**
   * The Mix view (design: Components → Tracks; AGENTS.md §8.6): one compact row per stem: the
   * instrument icon in the stem's hue, mute and solo, gain and pan knobs, then a dot waveform in
   * the stem's hue with a grey copy that wipes over it when the track is inaudible (rule 17). Rows
   * share the window's height. Lanes follow the shared timeline viewport (⌘-scroll or pinch to
   * zoom, horizontal scroll to pan). Scrubbing any lane moves the playhead directly (rule 14) and
   * rubber-bands past either end (rule 19). The MIDI button transcribes a track (basic-pitch, or
   * the drum classifier) into a MIDI lane under it, exportable as a .mid file (AGENTS.md §4.3).
   */
  let {
    project,
    snapshot,
    transcriber,
    view,
    onView,
    onSeek,
    onMix,
    onMixCommit,
    onProject,
    onModels,
  }: {
    project: Project;
    snapshot: TransportSnapshot;
    transcriber: Transcriber;
    /** The shared timeline viewport. */
    view: View;
    onView: (v: View) => void;
    onSeek: (frame: number) => void;
    /** A mix change to hear now (at most one per frame while a knob turns). */
    onMix: (track: number, params: MixParams) => void;
    /** A finished mix change, for undo. */
    onMixCommit: (track: number, before: MixParams, after: MixParams) => void;
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

  /** Shortest and tallest lane; rows share the window's height between these. */
  const LANE_MIN = 44;
  const LANE_MAX = 132;
  const ROW_GAP = 8;
  const ROW_PAD = 12;
  const RULER_H = 28;
  const RUBBER_LIMIT_PX = 24;

  const ICONS: Record<string, IconName> = { Vocals: "mic", Drums: "drums", Bass: "bass", Guitar: "guitar", Piano: "piano", Other: "waves" };
  const iconOf = (source: TrackSource): IconName => (source.kind === "Stem" ? ICONS[source.stem] : "waves");

  type Scrub =
    | { kind: "idle" }
    | { kind: "scrubbing"; seconds: number; overshootPx: number }
    | { kind: "settling"; seconds: number; overshootPx: number; cancel: () => void };
  let scrub = $state<Scrub>({ kind: "idle" });

  let width = $state(0);
  let height = $state(0);
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
  const midiRows = $derived(project.transcriptions.length);
  /** Lane height: the rows (reference vocal + tracks) share what the ruler and MIDI lanes leave. */
  const LANE_H = $derived.by(() => {
    const n = rows.length + 1;
    const free = height - RULER_H - midiRows * (MIDI_H + ROW_GAP) - n * (ROW_GAP + ROW_PAD);
    return Math.round(Math.min(LANE_MAX, Math.max(LANE_MIN, free / n)));
  });
  /** The visible part of the song, as fractions (for waveform levels). */
  const from = $derived(Math.max(0, view.start / duration));
  const to = $derived(Math.min(1, (view.start + view.span) / duration));
  /** x (px) of song time `s` in the lanes. */
  const x = (s: number) => timeToX(view, s, width);

  function audible(t: Track): boolean {
    return t.mix.mode === "Soloed" || (t.mix.mode === "Normal" && !anySolo);
  }

  function toggled(mode: TrackMode, target: "Muted" | "Soloed"): TrackMode {
    return mode === target ? "Normal" : target;
  }

  /** Playhead x in a lane: the clock (extrapolated per frame), or the scrub position while
   * scrubbing; moved by transform only (AGENTS.md §10.1). */
  const headX = (seconds: number): number | null => {
    const s = scrub.kind === "idle" ? seconds : scrub.seconds;
    const px = x(s) + (scrub.kind === "idle" ? 0 : scrub.overshootPx);
    return px < -2 || px > width + 2 ? null : px;
  };

  // Lanes are drawn only when what they show changes (never per frame).
  $effect(() => {
    if (width <= 0 || !rulerCanvas) return;
    const c = fit(rulerCanvas, width, RULER_H);
    if (!c) return;
    void view;
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
    void view;
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
    const levels = cachedLevels(t.peaks, columns, from, to);
    const h = LANE_H;
    return { width, key: [levels, t.mix.gain, col, width, h], draw: (c) => drawDotWave(c, width, h, levels, t.mix.gain, col) };
  };

  const midiArt = (t: Track): LaneArt => {
    const layer = layerOf(project, t.source);
    return {
      width,
      key: [layer, width, view],
      draw: (c) => {
        if (!layer) return;
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
    const seconds = Math.min(duration, Math.max(0, xToTime(view, clamped, rect.width)));
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

<div class="mix" bind:clientHeight={height} use:viewportGestures={{ get: () => view, set: onView, duration }}>
  <div class="row ruler">
    <div class="controls"></div>
    <div class="lane-box" bind:clientWidth={width}>
      {#snippet rulerLane()}<canvas bind:this={rulerCanvas} style:height="{RULER_H}px"></canvas>{/snippet}
      {@render lane(rulerLane, RULER_H, "Playhead")}
    </div>
  </div>

  <div class="row track" in:enter={{ delay: detailDelay(0) }}>
    <div class="controls">
      <span class="instrument" style:color={stemHue.Vocals} title="Reference vocal"><Icon name="mic" /></span>
      <span class="title">Reference</span>
      <span class="muted small">{project.analysis.reference_notes.length} notes</span>
    </div>
    <div class="lane-box">
      {#snippet refLane()}<canvas bind:this={refCanvas} use:reveal={revealKey} style:height="{LANE_H}px"></canvas>{/snippet}
      {@render lane(refLane, LANE_H, "Reference vocal, playhead")}
    </div>
  </div>

  {#each rows as { track: t, index: i }, row (t.name)}
    {@const hue = trackHue(t.source)}
    {@const muted = t.mix.mode === "Muted"}
    {@const soloed = t.mix.mode === "Soloed"}
    <div class="row track" in:enter={{ delay: detailDelay(row + 1) }}>
      <div class="controls">
        <span class="instrument" style:color={hue} title={t.name}><Icon name={iconOf(t.source)} /></span>
        <span class="title">{t.name}</span>
        <button
          class="press icon ms"
          class:on={muted}
          aria-pressed={muted}
          aria-label="Mute {t.name}"
          title="Mute"
          onclick={() => {
            const after = { ...t.mix, mode: toggled(t.mix.mode, "Muted") };
            onMix(i, after);
            onMixCommit(i, t.mix, after);
          }}><Icon name="mute" /></button
        >
        <button
          class="press icon ms"
          class:on={soloed}
          aria-pressed={soloed}
          aria-label="Solo {t.name}"
          title="Solo"
          onclick={() => {
            const after = { ...t.mix, mode: toggled(t.mix.mode, "Soloed") };
            onMix(i, after);
            onMixCommit(i, t.mix, after);
          }}><Icon name="solo" /></button
        >
        <Knob
          value={gainToKnob(t.mix.gain)}
          reset={gainToKnob(1)}
          label="{t.name} gain"
          colour={hue}
          format={(v) => formatDb(knobToGain(v))}
          onChange={(v) => onMix(i, { ...t.mix, gain: knobToGain(v) })}
          onCommit={(a, b) => onMixCommit(i, { ...t.mix, gain: knobToGain(a) }, { ...t.mix, gain: knobToGain(b) })}
        />
        <Knob
          value={panToKnob(t.mix.pan)}
          reset={0.5}
          bipolar
          label="{t.name} pan"
          colour={hue}
          format={(v) => formatPan(knobToPan(v))}
          onChange={(v) => onMix(i, { ...t.mix, pan: knobToPan(v) })}
          onCommit={(a, b) => onMixCommit(i, { ...t.mix, pan: knobToPan(a) }, { ...t.mix, pan: knobToPan(b) })}
        />
        {#if !layerOf(project, t.source)}
          <button
            class="press icon quiet"
            disabled={jobOf(t).kind === "transcribing"}
            aria-label="Transcribe {t.name} to MIDI"
            title={jobOf(t).kind === "transcribing" ? "Transcribing…" : "Transcribe to MIDI"}
            onclick={() => transcribe(t)}><Icon name="midi" /></button
          >
        {/if}
      </div>
      <div class="lane-box">
        {#snippet trackLane()}
          <canvas use:reveal={revealKey} use:art={waveArt(t, hue)} style:height="{LANE_H}px"></canvas>
          <canvas class="mask" class:on={!audible(t)} use:art={waveArt(t, colour.disabled)} style:height="{LANE_H}px"></canvas>
        {/snippet}
        {@render lane(trackLane, LANE_H, `${t.name} lane, playhead`)}
      </div>
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
    height: calc(100vh - 210px);
    min-height: 360px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 16px;
    box-sizing: border-box;
    padding: 0 12px 0 12px;
  }
  .track {
    padding-top: 6px;
    padding-bottom: 6px;
    background: var(--c-surface);
    border-radius: 12px;
  }
  .controls {
    width: 316px;
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
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
    padding-left: 8px;
    box-sizing: border-box;
  }
  .notice {
    gap: 12px;
    padding-left: 56px;
  }
  .spacer {
    flex: 1;
  }
  .small {
    font-size: 13px;
    line-height: 18px;
  }
  .instrument {
    width: 36px;
    height: 44px;
    display: grid;
    place-items: center;
    flex: none;
  }
  .title {
    flex: 1;
    min-width: 0;
    font-size: 14px;
    line-height: 20px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ms {
    color: var(--c-muted);
    transition:
      background-color var(--dur-colour) var(--ease-out),
      color var(--dur-colour) var(--ease-out),
      transform var(--dur-press) var(--ease-out);
  }
  .ms.on {
    background: var(--c-text);
    color: var(--c-ground);
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
</style>
