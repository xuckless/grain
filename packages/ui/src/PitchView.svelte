<script lang="ts">
  import type { EditOp, EditSource, EngineHost, PitchEdit, PitchEditView, TransportSnapshot } from "@workstation/engine-api";
  import DotSlider from "./DotSlider.svelte";
  import Menu from "./Menu.svelte";
  import { formatPercent, formatSemitones } from "./autotune";
  import Takes from "./Takes.svelte";
  import { dot, fit } from "./grain";
  import { enter, reveal, rubberBand, springBack } from "./motion";
  import {
    advance,
    clamp,
    commit,
    decideDrag,
    defaultSource,
    dragged,
    formatShift,
    historyOf,
    hitTest,
    initialWindow,
    inScale,
    pan,
    redo,
    select,
    selectAll,
    selectedIds,
    sourceFromId,
    sourceId,
    sourceOptions,
    targetIds,
    undo,
    zoom,
    type Drag,
    type History,
    type Loading,
    type RenderState,
    type Selection,
    type Window,
  } from "./pitch";
  import { applyTake } from "./recorder";
  import { playhead } from "./clock";
  import { errorMessage, type Session } from "./state";
  import { colour, stemHue } from "./theme";

  /**
   * The Pitch view (AGENTS.md §7.1): a Melodyne-style editor for one take or the vocal stem.
   * Notes are dot-grain blobs on a piano roll, sized by loudness, with the pitch line over them.
   * Drag a note up or down (snapped; ⌥ for free cents), left or right (timing), or by an edge
   * (stretch). The tools reshape the selected notes, or all of them when none is selected. The
   * edits are kept with the song and rendered into a new take, so nothing is overwritten.
   */
  let {
    host,
    session,
    snapshot,
    onSeek,
  }: { host: EngineHost; session: Session; snapshot: TransportSnapshot; onSeek: (frame: number) => void } = $props();

  // The host is fixed for the app's lifetime.
  // svelte-ignore state_referenced_locally
  const editor = host.pitchEditor;

  const HEIGHT = 420;
  /** Rubber-band reach past the song's ends while panning, px. */
  const RUBBER_LIMIT_PX = 48;
  /** Rows of grain a fully loud frame stacks to, either side of the note line. */
  const BLOB_ROWS = 3;

  const project = $derived(session.project);
  const duration = $derived(project.frames / project.sample_rate);
  const source = $derived(session.pitchSource ?? defaultSource(project));
  const sid = $derived(source === null ? null : sourceId(source));
  const hue = $derived(source?.kind === "Take" ? colour.text : stemHue.Vocals);

  // ---- Loading a source -----------------------------------------------------------------------

  type Loaded = { view: PitchEditView; history: History; edited: (number | null)[]; mask: number };
  let loading = $state<Loading<Loaded>>({ kind: "loading" });
  let win = $state<Window>({ from: 0, to: 1, lo: 48, hi: 72 });
  let selection = $state<Selection>({ kind: "none" });

  $effect(() => {
    const id = sid;
    if (id === null) return;
    const s = sourceFromId(id);
    loading = { kind: "loading" };
    selection = { kind: "none" };
    editor.open(s).then(
      (view) => {
        if (sid !== id) return;
        loading = { kind: "ready", value: { view, history: historyOf(view.edit), edited: view.edited, mask: view.scale_mask } };
        win = initialWindow(view.edit, duration);
      },
      (e) => sid === id && (loading = { kind: "failed", message: errorMessage(e) }),
    );
  });

  const ready = $derived(loading.kind === "ready" ? loading.value : null);
  const edit = $derived(ready?.history.present ?? null);

  type Notice = { kind: "none" } | { kind: "error"; message: string };
  let notice = $state<Notice>({ kind: "none" });

  function dirty() {
    session.touch();
  }

  /** Applies one edit through the engine and records it for undo (`key` coalesces a slider). */
  async function run(op: EditOp, key: string | null = null) {
    if (loading.kind !== "ready") return;
    const before = loading.value;
    try {
      const u = await editor.apply($state.snapshot(before.history.present) as PitchEdit, op);
      if (loading.kind !== "ready") return;
      loading = { kind: "ready", value: { ...loading.value, history: commit(loading.value.history, u.edit, key), edited: u.edited, mask: u.scale_mask } };
      notice = { kind: "none" };
      dirty();
    } catch (e) {
      notice = { kind: "error", message: errorMessage(e) };
    }
  }

  /** Moves through history (undo/redo): the engine keeps the restored list. */
  async function travel(next: History | null) {
    if (!next || loading.kind !== "ready") return;
    try {
      const u = await editor.set($state.snapshot(next.present) as PitchEdit);
      if (loading.kind !== "ready") return;
      loading = { kind: "ready", value: { ...loading.value, history: next, edited: u.edited, mask: u.scale_mask } };
      dirty();
    } catch (e) {
      notice = { kind: "error", message: errorMessage(e) };
    }
  }

  // ---- Controls -------------------------------------------------------------------------------

  type SnapChoice = "Scale" | "Semitone";
  let snap = $state<SnapChoice>("Scale");
  type Tool = "Edit" | "Split";
  let tool = $state<Tool>("Edit");
  /** Last "Pitch centre" amount used (it isn't stored per note). */
  let centreAmount = $state(1);

  const targets = $derived(edit ? targetIds(selection, edit) : []);
  const first = $derived(edit?.notes.find((n) => n.id === targets[0]) ?? null);
  const chosen = $derived(edit ? selectedIds(selection, edit) : []);
  const canMerge = $derived(edit !== null && chosen.length === 1 && edit.notes.findIndex((n) => n.id === chosen[0]) < edit.notes.length - 1);

  // ---- Rendering and listening ----------------------------------------------------------------

  let render = $state<RenderState>({ kind: "idle" });

  async function renderTake() {
    if (source === null || render.kind === "rendering") return;
    render = { kind: "rendering" };
    // Keep editing this source: the new take would otherwise become the default.
    session.pitchSource = source;
    try {
      const added = await editor.render(source, $state.snapshot(session.settings));
      applyTake(session, added);
      render = { kind: "rendered", take: added.take };
    } catch (e) {
      render = { kind: "failed", message: errorMessage(e) };
    }
  }


  // ---- Canvas ---------------------------------------------------------------------------------

  let canvas = $state<HTMLCanvasElement | undefined>();
  let surface = $state<HTMLDivElement | undefined>();
  let width = $state(0);
  let drag = $state<Drag>({ kind: "idle" });
  let cursor = $state("default");

  const pxPerSecond = $derived(width / Math.max(1e-6, win.to - win.from));
  const pxPerSemitone = $derived(HEIGHT / Math.max(1e-6, win.hi - win.lo));
  const overPx = $derived(drag.kind === "panning" ? drag.overPx : 0);
  const x = (s: number) => (s - win.from) * pxPerSecond - overPx;
  const y = (m: number) => HEIGHT - (m - win.lo) * pxPerSemitone;
  const secondsAt = (px: number) => win.from + px / pxPerSecond;
  const midiAt = (py: number) => win.lo + (HEIGHT - py) / pxPerSemitone;

  $effect(() => {
    if (!canvas || width <= 0 || !ready || !edit) return;
    const c = fit(canvas, width, HEIGHT);
    if (!c) return;
    const { view, edited, mask } = ready;
    const hop = view.hop_seconds;
    const selected = new Set(chosen);

    // Scale rows: dotted lines on the scale's notes, brighter on C.
    for (let m = Math.ceil(win.lo); m <= Math.floor(win.hi); m++) {
      if (!inScale(mask, m)) continue;
      const col = m % 12 === 0 ? colour.disabled : colour.raised;
      for (let gx = 4; gx < width; gx += 10) dot(c, gx, y(m), 0.7, col);
    }

    // The pitch as sung: faint grains.
    const a = Math.max(0, Math.floor(win.from / hop)), b = Math.min(view.sung.length, Math.ceil(win.to / hop));
    for (let i = a; i < b; i++) {
      const m = view.sung[i];
      if (m !== null && m !== undefined) dot(c, x(i * hop), y(m), 0.8, colour.disabled);
    }

    // Notes: a blob of grain along the note line, thick where the note is loud.
    for (const n of edit.notes) {
      const d = dragged(n, drag);
      if (d.end < win.from || d.start > win.to) continue;
      const col = selected.has(n.id) ? colour.text : hue;
      const steps = Math.max(1, Math.floor((x(d.end) - x(d.start)) / 4));
      for (let k = 0; k <= steps; k++) {
        const t = d.start + ((d.end - d.start) * k) / steps;
        const src = n.start + ((t - d.start) / Math.max(1e-6, d.end - d.start)) * (n.end - n.start);
        const amp = view.amplitude[Math.round(src / hop)] ?? 0;
        const rows = Math.max(0, Math.round(amp * BLOB_ROWS));
        for (let r = -rows; r <= rows; r++) dot(c, x(t), y(d.pitch) + r * 3.2, 1.3 - Math.abs(r) * 0.15, col);
      }
    }

    // The pitch after the edits.
    if (drag.kind === "idle" || drag.kind === "panning" || drag.kind === "pressing") {
      for (let i = a; i < Math.min(edited.length, b); i++) {
        const m = edited[i];
        if (m !== null && m !== undefined) dot(c, x(i * hop), y(m), 1, hue);
      }
    }

  });

  /** The playhead, moved per frame by transform (never a redraw of the roll). */
  const headAt = (seconds: number): number | null => {
    const px = x(seconds);
    return px >= 0 && px <= width ? px : null;
  };

  function local(e: PointerEvent | WheelEvent): { px: number; py: number } {
    const r = surface!.getBoundingClientRect();
    return { px: e.clientX - r.left, py: e.clientY - r.top };
  }

  /** Song time on the sung timeline of note `id` at output time `t` (for splitting). */
  function sungTime(id: number, t: number): number {
    const n = edit!.notes.find((m) => m.id === id)!;
    return n.start + ((t - n.out_start) / Math.max(1e-6, n.out_end - n.out_start)) * (n.end - n.start);
  }

  let settle: () => void = () => {};

  function down(e: PointerEvent) {
    if (!edit || e.button !== 0) return;
    settle();
    surface!.setPointerCapture(e.pointerId);
    const { px, py } = local(e);
    const hit = hitTest(edit, secondsAt(px), midiAt(py), pxPerSecond);
    const extend = e.shiftKey || e.metaKey || e.ctrlKey;
    if (hit && tool === "Split") {
      void run({ kind: "Split", id: hit.id, at: sungTime(hit.id, secondsAt(px)) });
      return;
    }
    selection = select(selection, hit?.id ?? null, extend);
    drag = hit ? { kind: "pressing", hit, x0: px, y0: py } : { kind: "panning", x0: px, y0: py, from: { ...win }, overPx: 0 };
  }

  function move(e: PointerEvent) {
    if (!edit) return;
    const { px, py } = local(e);
    switch (drag.kind) {
      case "idle": {
        const hit = hitTest(edit, secondsAt(px), midiAt(py), pxPerSecond);
        cursor = tool === "Split" && hit ? "crosshair" : hit ? (hit.part === "body" ? "grab" : "ew-resize") : "default";
        return;
      }
      case "pressing": {
        drag = decideDrag(drag.hit, drag.x0, drag.y0, px, py, chosen) ?? drag;
        return;
      }
      case "pitch":
      case "time":
      case "stretch":
        drag = advance(drag, px, py, pxPerSecond, pxPerSemitone);
        return;
      case "panning": {
        const dx = (px - drag.x0) / pxPerSecond, dy = (py - drag.y0) / pxPerSemitone;
        const p = pan(drag.from, -dx, dy, duration);
        win = p.window;
        drag = { ...drag, overPx: rubberBand(p.over * pxPerSecond, RUBBER_LIMIT_PX) };
        return;
      }
    }
  }

  function up(e: PointerEvent) {
    const d = drag;
    drag = { kind: "idle" };
    if (!edit) return;
    switch (d.kind) {
      case "pitch":
        void run({ kind: "MovePitch", ids: d.ids, semitones: d.semitones, snap: e.altKey ? "Free" : snap });
        return;
      case "time": {
        const n = edit.notes.find((m) => m.id === d.id);
        if (n) void run({ kind: "MoveTime", id: d.id, out_start: n.out_start + d.seconds });
        return;
      }
      case "stretch": {
        const n = edit.notes.find((m) => m.id === d.id);
        if (!n) return;
        const s = dragged(n, d);
        void run({ kind: "Stretch", id: d.id, out_start: s.start, out_end: s.end });
        return;
      }
      case "pressing": {
        // A click without a drag also moves the playhead there.
        const { px } = local(e);
        onSeek(Math.floor(clamp(secondsAt(px), 0, duration - 1 / project.sample_rate) * project.sample_rate));
        return;
      }
      case "panning": {
        if (d.overPx !== 0) {
          drag = d;
          settle = springBack(
            d.overPx,
            (v) => drag.kind === "panning" && (drag = { ...drag, overPx: v }),
            () => (drag = { kind: "idle" }),
          );
        } else if (Math.hypot(local(e).px - d.x0, local(e).py - d.y0) < 3) {
          onSeek(Math.floor(clamp(secondsAt(local(e).px), 0, duration - 1 / project.sample_rate) * project.sample_rate));
        }
        return;
      }
    }
  }

  // Wheel: scroll in time (⇧: in pitch), ⌘/Ctrl zooms. Registered non-passive to keep the page still.
  $effect(() => {
    const el = surface;
    if (!el) return;
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      if (e.metaKey || e.ctrlKey) {
        win = zoom(win, Math.exp(e.deltaY * 0.002), secondsAt(local(e).px), duration);
      } else if (e.shiftKey) {
        win = pan(win, 0, -e.deltaY / pxPerSemitone / 4, duration).window;
      } else {
        win = pan(win, (e.deltaX + e.deltaY) / pxPerSecond, 0, duration).window;
      }
    };
    el.addEventListener("wheel", wheel, { passive: false });
    return () => el.removeEventListener("wheel", wheel);
  });

  // Keys: ⌘Z / ⇧⌘Z undo and redo, ⌘A selects all, Delete resets the selected notes (never
  // removes them), Escape clears the selection.
  $effect(() => {
    const keys = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || !ready || !edit) return;
      const mod = e.metaKey || e.ctrlKey;
      const k = e.key.toLowerCase();
      if (mod && k === "z") {
        e.preventDefault();
        void travel(e.shiftKey ? redo(ready.history) : undo(ready.history));
      } else if (mod && k === "y") {
        e.preventDefault();
        void travel(redo(ready.history));
      } else if (mod && k === "a") {
        e.preventDefault();
        selection = selectAll(edit);
      } else if ((e.key === "Delete" || e.key === "Backspace") && chosen.length > 0) {
        e.preventDefault();
        void run({ kind: "Reset", ids: chosen });
      } else if (e.key === "Escape") {
        selection = { kind: "none" };
      }
    };
    window.addEventListener("keydown", keys);
    return () => window.removeEventListener("keydown", keys);
  });

  const sources = $derived(sourceOptions(project));
</script>

<section class="pitch">
  {#if source === null}
    <div class="caption">
      <h2>Nothing to edit yet</h2>
      <p class="muted">Record or import a take in the Score tab, or import a song with a vocal, to edit its notes here.</p>
    </div>
  {:else}
    <div class="controls">
      <Menu label="Source" value={sid ?? ""} options={sources} onPick={(id) => (session.pitchSource = sourceFromId(id))} />
      <Menu label="Snap" value={snap} options={[{ id: "Scale", label: "Scale notes" }, { id: "Semitone", label: "Semitones" }]} onPick={(s) => (snap = s)} />
      <Menu label="Tool" value={tool} options={[{ id: "Edit", label: "Move and stretch" }, { id: "Split", label: "Split notes" }]} onPick={(t) => (tool = t)} />
      <span class="spacer"></span>
      <button class="press quiet" disabled={!ready || ready.history.past.length === 0} onclick={() => ready && void travel(undo(ready.history))}>Undo</button>
      <button class="press quiet" disabled={!ready || ready.history.future.length === 0} onclick={() => ready && void travel(redo(ready.history))}>Redo</button>
    </div>

    {#if loading.kind === "loading"}
      <p class="muted">Finding the notes</p>
    {:else if loading.kind === "failed"}
      <p class="muted">Couldn't open it: {loading.message}</p>
    {:else if edit}
      <div
        class="roll"
        bind:this={surface}
        bind:clientWidth={width}
        style:cursor={drag.kind === "pitch" ? "ns-resize" : drag.kind === "time" ? "grabbing" : drag.kind === "stretch" ? "ew-resize" : cursor}
        role="application"
        aria-label="Notes on a piano roll: drag a note up or down to change its pitch, sideways to move it, by an edge to stretch it"
        onpointerdown={down}
        onpointermove={move}
        onpointerup={up}
        onpointercancel={() => (drag = { kind: "idle" })}
      >
        <canvas bind:this={canvas} use:reveal={sid} style:height="{HEIGHT}px"></canvas>
        <div class="playhead" use:playhead={headAt} style:height="{HEIGHT}px"></div>
      </div>

      <div class="readout mono muted">
        {#if chosen.length === 0}
          {edit.notes.length} notes. Tools apply to all of them; click a note to select it (⇧ for more).
        {:else if first}
          {chosen.length} selected · {formatShift(first.pitch - first.centre)}{first.out_start !== first.start || first.out_end !== first.end ? " · moved" : ""}
        {/if}
      </div>

      <div class="tools" in:enter={{ detail: true }}>
        <button class="press primary" onclick={() => void run({ kind: "Correct", ids: targets, centre: 1, drift: 0.5 })}>Correct pitch</button>
        <button class="press" disabled={!canMerge} onclick={() => void run({ kind: "Merge", id: chosen[0] })}>Merge with next</button>
        <button class="press quiet" disabled={chosen.length === 0} onclick={() => void run({ kind: "Reset", ids: chosen })}>Reset</button>
      </div>

      <div class="sliders">
        <label class="slider">
          <span class="label">Pitch centre</span>
          <DotSlider value={centreAmount} min={0} max={1} step={0.01} width={180} label="Pitch centre correction" readout={formatPercent(centreAmount)} onChange={(v) => { centreAmount = v; void run({ kind: "Correct", ids: targets, centre: v, drift: null }, `centre:${targets.join()}`); }} />
        </label>
        <label class="slider">
          <span class="label">Pitch drift</span>
          <DotSlider value={1 - (first?.drift ?? 1)} min={0} max={1} step={0.01} width={180} label="Pitch drift correction" readout={formatPercent(1 - (first?.drift ?? 1))} onChange={(v) => void run({ kind: "Correct", ids: targets, centre: null, drift: v }, `drift:${targets.join()}`)} />
        </label>
        <label class="slider">
          <span class="label">Vibrato</span>
          <DotSlider value={first?.modulation ?? 1} min={0} max={2} step={0.01} width={180} label="Vibrato amount" readout={formatPercent(first?.modulation ?? 1)} onChange={(v) => void run({ kind: "SetModulation", ids: targets, modulation: v }, `mod:${targets.join()}`)} />
        </label>
        <label class="slider">
          <span class="label">Glide in</span>
          <DotSlider value={(first?.transition ?? 0) * 1000} min={0} max={500} step={5} width={180} label="Glide into the note" readout={`${Math.round((first?.transition ?? 0) * 1000)} ms`} onChange={(v) => void run({ kind: "SetTransition", ids: targets, seconds: v / 1000 }, `glide:${targets.join()}`)} />
        </label>
        <label class="slider">
          <span class="label">Formant</span>
          <DotSlider value={first?.formant ?? 0} min={-5} max={5} step={0.1} width={180} label="Formant shift" readout={formatSemitones(first?.formant ?? 0)} onChange={(v) => void run({ kind: "SetFormant", ids: targets, semitones: v }, `formant:${targets.join()}`)} />
        </label>
      </div>

      <div class="controls">
        <button class="press primary big" onclick={renderTake} disabled={render.kind === "rendering"}>
          {render.kind === "rendering" ? "Rendering the edits" : "Render as a take"}
        </button>
      </div>
      {#if render.kind === "rendered"}
        {@const id = render.take}
        <p class="muted" in:enter>Added “{project.takes.find((t) => t.id === id)?.name}”, kept with the song and scored. Compare it with the source under Your takes.</p>
      {:else if render.kind === "failed"}
        <p class="muted" in:enter>Couldn't render: {render.message}</p>
      {/if}
      {#if notice.kind === "error"}
        <p class="muted">{notice.message}</p>
      {/if}
    {/if}
  {/if}
  <Takes {host} {session} empty="Record or import a take in the Score tab; rendered edits appear here too." />
</section>

<style>
  .pitch {
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  .controls,
  .tools {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
  }
  .roll {
    position: relative;
    background: var(--c-surface);
    border-radius: 12px;
    touch-action: none;
    overflow: hidden;
  }
  .roll:focus-visible {
    outline: 2px solid var(--c-text);
    outline-offset: 2px;
  }
  canvas {
    display: block;
    width: 100%;
  }
  .readout {
    font-size: 13px;
    line-height: 18px;
    margin-top: -12px;
  }
  .sliders {
    display: flex;
    gap: 28px;
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
