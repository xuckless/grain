<script lang="ts">
  import type { FrameComparison, NoteResult, Project, ScoreReport } from "@workstation/engine-api";

  let { project, report, onSeek }: { project: Project; report: ScoreReport; onSeek: (seconds: number) => void } = $props();

  const ROLL = 260;
  const STRIP = 70;
  /** Cents shown at the top/bottom of the error strip. */
  const STRIP_RANGE = 100;

  let canvas: HTMLCanvasElement | undefined = $state();
  let width = $state(800);
  const height = ROLL + STRIP;
  const duration = $derived(project.frames / project.sample_rate);

  function css(name: string): string {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  }

  function noteColor(result: NoteResult, good: string, warn: string, bad: string): string {
    switch (result.kind) {
      case "Missed":
        return bad;
      case "Sung":
        return result.hit_ratio >= 0.6 ? good : warn;
    }
  }

  function frameColor(f: FrameComparison, good: string, bad: string): string | undefined {
    switch (f.kind) {
      case "Hit":
        return good;
      case "Miss":
        return bad;
      case "ReferenceOnly":
      case "UserOnly":
      case "Silent":
        return undefined;
    }
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
    const [line, muted, good, warn, bad, text] = ["--line", "--muted", "--good", "--warn", "--bad", "--text"].map(css);

    const notes = report.notes.map((n) => n.note);
    const lo = Math.min(...notes.map((n) => n.midi)) - 3;
    const hi = Math.max(...notes.map((n) => n.midi)) + 3;
    const x = (t: number) => (t / duration) * width;
    const y = (midi: number) => ROLL - ((midi - lo) / (hi - lo)) * ROLL;
    const rowHeight = ROLL / (hi - lo);

    // Semitone grid; MIDI multiples of 12 are C.
    g.font = "10px system-ui";
    for (let m = Math.ceil(lo); m <= hi; m++) {
      g.fillStyle = m % 12 === 0 ? muted : line;
      g.fillRect(0, y(m), width, 1);
      if (m % 12 === 0) g.fillText(`MIDI ${m}`, 2, y(m) - 2);
    }

    // Reference notes colored by how they were sung.
    for (const n of report.notes) {
      g.fillStyle = noteColor(n.result, good, warn, bad);
      g.globalAlpha = 0.35;
      g.fillRect(x(n.note.start), y(n.note.midi) - rowHeight / 2, Math.max(2, x(n.note.end) - x(n.note.start)), rowHeight);
    }
    g.globalAlpha = 1;

    // The user's pitch (already shifted onto the reference timeline).
    report.user_pitch.frames.forEach((f, i) => {
      if (f.kind !== "Voiced") return;
      const c = frameColor(report.frames[i], good, bad);
      g.fillStyle = c ?? muted;
      g.fillRect(x(i * report.hop_seconds), y(f.midi) - 1, 2, 2);
    });

    // Cents error strip.
    const top = ROLL + 8;
    const mid = top + (STRIP - 8) / 2;
    g.fillStyle = line;
    g.fillRect(0, mid, width, 1);
    g.fillStyle = muted;
    g.fillText(`±${STRIP_RANGE}¢`, 2, top + 9);
    report.frames.forEach((f, i) => {
      if (f.kind !== "Hit" && f.kind !== "Miss") return;
      g.fillStyle = f.kind === "Hit" ? good : bad;
      const h = (Math.max(-STRIP_RANGE, Math.min(STRIP_RANGE, f.cents)) / STRIP_RANGE) * ((STRIP - 8) / 2);
      g.fillRect(x(i * report.hop_seconds), mid - Math.max(0, h), 1, Math.max(1, Math.abs(h)));
    });
    g.fillStyle = text;
  });

  function click(event: MouseEvent) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const t = ((event.clientX - rect.left) / rect.width) * duration;
    const note = report.notes.find((n) => n.note.start <= t && t <= n.note.end);
    onSeek(note ? note.note.start : t);
  }
</script>

<div bind:clientWidth={width}>
  <canvas bind:this={canvas} style:width="{width}px" style:height="{height}px" onclick={click}></canvas>
</div>
<p class="legend">Bars: reference notes (green = hit, yellow = partly, red = missed). Dots: your pitch. Strip: cents sharp (up) / flat (down). Click a note to play from it.</p>

<style>
  canvas {
    display: block;
    cursor: pointer;
  }
  .legend {
    color: var(--muted);
    font-size: 12px;
    margin: 0;
  }
</style>
