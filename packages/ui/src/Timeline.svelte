<script lang="ts">
  import type { MixParams, Project, TrackMode, TransportSnapshot } from "@workstation/engine-api";

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

  const RULER = 22;
  const NOTES_LANE = 64;
  const TRACK = 64;

  let canvas: HTMLCanvasElement | undefined = $state();
  let width = $state(800);

  const duration = $derived(project.frames / project.sample_rate);
  const height = $derived(RULER + NOTES_LANE + TRACK * project.tracks.length);
  const playhead = $derived.by(() => {
    const t = snapshot.transport;
    switch (t.kind) {
      case "Stopped":
        return 0;
      case "Playing":
      case "Paused":
        return t.frame / snapshot.sample_rate;
    }
  });

  function toggled(mode: TrackMode, target: "Muted" | "Soloed"): TrackMode {
    return mode === target ? "Normal" : target;
  }

  function css(name: string): string {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  $effect(() => {
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(width * dpr);
    canvas.height = Math.round(height * dpr);
    const g = canvas.getContext("2d");
    if (!g) return;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, width, height);
    const x = (seconds: number) => (seconds / duration) * width;
    const [line, muted, wave, accent, text] = ["--line", "--muted", "--wave", "--accent", "--text"].map(css);

    // Ruler: every 4th beat labelled with its bar number.
    g.fillStyle = muted;
    g.font = "10px system-ui";
    project.analysis.tempo.beats.forEach((b, i) => {
      const bx = x(b);
      g.fillRect(bx, i % 4 === 0 ? 8 : 15, 1, i % 4 === 0 ? 14 : 7);
      if (i % 16 === 0) g.fillText(String(i / 4 + 1), bx + 3, 10);
    });

    // Reference vocal notes (piano roll squeezed into one lane).
    const notes = project.analysis.reference_notes;
    if (notes.length > 0) {
      const lo = Math.min(...notes.map((n) => n.midi)) - 1;
      const hi = Math.max(...notes.map((n) => n.midi)) + 1;
      const y = (midi: number) => RULER + NOTES_LANE - 4 - ((midi - lo) / (hi - lo)) * (NOTES_LANE - 8);
      g.fillStyle = accent;
      for (const n of notes) g.fillRect(x(n.start), y(n.midi) - 2, Math.max(1, x(n.end) - x(n.start)), 4);
    }

    // Track waveforms from the min/max peak pairs.
    project.tracks.forEach((track, row) => {
      const top = RULER + NOTES_LANE + row * TRACK;
      const mid = top + TRACK / 2;
      g.fillStyle = line;
      g.fillRect(0, top, width, 1);
      g.fillStyle = wave;
      const pairs = track.peaks.length / 2;
      const pairsPerSecond = project.sample_rate / project.samples_per_peak;
      for (let px = 0; px < width; px++) {
        const a = Math.floor(((px / width) * duration) * pairsPerSecond);
        const b = Math.min(pairs, Math.max(a + 1, Math.floor((((px + 1) / width) * duration) * pairsPerSecond)));
        let lo = 0;
        let hi = 0;
        for (let i = a; i < b; i++) {
          lo = Math.min(lo, track.peaks[2 * i]);
          hi = Math.max(hi, track.peaks[2 * i + 1]);
        }
        const scale = (TRACK / 2 - 4) * track.mix.gain;
        g.fillRect(px, mid - hi * scale, 1, Math.max(1, (hi - lo) * scale));
      }
    });

    g.fillStyle = text;
    g.fillRect(x(playhead), 0, 1.5, height);
  });

  function seek(event: MouseEvent) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const seconds = ((event.clientX - rect.left) / rect.width) * duration;
    onSeek(Math.max(0, Math.floor(seconds * snapshot.sample_rate)));
  }
</script>

<div class="timeline">
  <div class="headers">
    <div style:height="{RULER}px"></div>
    <div class="lane-title" style:height="{NOTES_LANE}px">Reference vocal</div>
    {#each project.tracks as track, i (track.name)}
      <div class="track" style:height="{TRACK}px">
        <div class="row">
          <span class="name">{track.name}</span>
          <button
            class:on={track.mix.mode === "Muted"}
            title="Mute"
            onclick={() => onMix(i, { ...track.mix, mode: toggled(track.mix.mode, "Muted") })}>M</button
          >
          <button
            class:on={track.mix.mode === "Soloed"}
            title="Solo"
            onclick={() => onMix(i, { ...track.mix, mode: toggled(track.mix.mode, "Soloed") })}>S</button
          >
        </div>
        <div class="row">
          <input
            type="range"
            min="0"
            max="2"
            step="0.01"
            title="Gain"
            value={track.mix.gain}
            oninput={(e) => onMix(i, { ...track.mix, gain: Number(e.currentTarget.value) })}
          />
          <input
            type="range"
            min="-1"
            max="1"
            step="0.01"
            title="Pan"
            value={track.mix.pan}
            oninput={(e) => onMix(i, { ...track.mix, pan: Number(e.currentTarget.value) })}
          />
        </div>
      </div>
    {/each}
  </div>
  <div class="lanes" bind:clientWidth={width}>
    <canvas bind:this={canvas} style:width="{width}px" style:height="{height}px" onclick={seek}></canvas>
  </div>
</div>

<style>
  .timeline {
    display: grid;
    grid-template-columns: 190px 1fr;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 8px;
    overflow: hidden;
  }
  .headers {
    border-right: 1px solid var(--line);
  }
  .lane-title,
  .track {
    box-sizing: border-box;
    border-top: 1px solid var(--line);
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 4px;
  }
  .lane-title {
    color: var(--muted);
  }
  .row {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .name {
    flex: 1;
    font-weight: 600;
  }
  .row button {
    padding: 0 6px;
    min-width: 24px;
  }
  .row button.on {
    background: var(--accent);
    color: white;
    border-color: var(--accent);
  }
  .row input[type="range"] {
    width: 80px;
    padding: 0;
  }
  .lanes {
    min-width: 0;
  }
  canvas {
    display: block;
    cursor: pointer;
  }
</style>
