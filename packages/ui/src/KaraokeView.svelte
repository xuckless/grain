<script lang="ts">
  import type { EngineHost, LiveFrame, PlayerCommand, TransportSnapshot } from "@workstation/engine-api";
  import BackingMenu from "./BackingMenu.svelte";
  import BottomAction from "./BottomAction.svelte";
  import Grain from "./Grain.svelte";
  import Icon from "./Icon.svelte";
  import LyricsPanel from "./LyricsPanel.svelte";
  import SettingsControls from "./SettingsControls.svelte";
  import TakePicker from "./TakePicker.svelte";
  import Toolbar from "./Toolbar.svelte";
  import { listener, services } from "./context";
  import { frameFeedback, type Feedback } from "./feedback";
  import { PitchTrail, type TrailPoint } from "./grain";
  import {
    addFrames,
    displayMidi,
    endRun,
    inTune,
    isLive,
    karaokeNext,
    newPerformance,
    noteAt,
    observeKaraoke,
    performanceTotals,
    type Karaoke,
    type KaraokeEvent,
  } from "./karaoke";
  import { currentLine, sungFraction } from "./lyrics";
  import { enter } from "./motion";
  import { applyTake, armTake, backingHint, cancelTake, eachFrame, effectiveBacking, finishTake, runCommands } from "./recorder";
  import { clock } from "./clock";
  import Takes from "./Takes.svelte";
  import { errorMessage, formatTime, transportSeconds, type Session } from "./state";
  import { beatsLeft } from "./takes";

  /**
   * The Karaoke view (AGENTS.md §6): the song's reference notes scroll through a pitch lane
   * while the live microphone pitch draws a pitch trail over them, coloured hit / near / miss
   * by the same per-frame rule the offline scorer uses. Notes fill with their verdict as the
   * song passes them; hits burst and build a streak. Two lyric lines wipe word by word. Every
   * run (count-in to pause or finish) is saved as a take and scored offline, so it shows in Score.
   * Seek by dragging the lane, clicking the ruler or clicking a lyric line; Sing starts with a
   * 2 s pre-roll. With a loop set (A, then B), every pass is saved as its own take and the next
   * pass counts in again from A (AGENTS.md §8.6).
   */
  let { host, session, snapshot }: { host: EngineHost; session: Session; snapshot: TransportSnapshot } = $props();

  const { burst, toast } = services();
  // The host is fixed for the app's lifetime.
  // svelte-ignore state_referenced_locally
  const mic = host.microphone;

  const project = $derived(session.project);
  const notes = $derived(project.analysis.reference_notes);
  /** Copied: the session's settings are a reactive proxy, which hosts can't post or clone. */
  const settings = () => ({ ...session.settings });
  const run = (commands: PlayerCommand[]) => runCommands(host, commands);

  // ---- Song clock -----------------------------------------------------------------------------
  // The lane scrolls with the shared clock (pushed transport, extrapolated per frame).

  const songClock = (now: number) => clock.seconds(now);
  const seconds = $derived(transportSeconds(snapshot));

  // ---- Performance ----------------------------------------------------------------------------

  // A song's notes never change (taking a take replaces the project object, not its analysis),
  // so the lane and the performance are made once.
  // svelte-ignore state_referenced_locally
  let performance_ = newPerformance(notes);
  // svelte-ignore state_referenced_locally
  const trail = new PitchTrail(notes, songClock, 0.3);
  let shown = $state({ score: 0, streak: 0, best: 0, takes: 0 });
  let lane = $state<HTMLElement | undefined>();
  /** Last reference note sung against, so octave folding holds between notes. */
  let folding: number | null = null;

  function refresh() {
    const p = performance_;
    shown = { score: inTune(performanceTotals(p)), streak: p.streak, best: p.best, takes: p.takes.length };
  }

  function celebrate(judged: { note: number; fill: Feedback }[]) {
    if (!lane || !judged.some((j) => j.fill === "hit")) return;
    const r = lane.getBoundingClientRect();
    const head = trail.headPoint();
    burst.fire(r.left + head.x, r.top + head.y);
  }

  function points(frames: readonly LiveFrame[]): TrailPoint[] {
    const out: TrailPoint[] = [];
    for (const f of frames) {
      if (f.pitch.kind !== "Voiced") continue;
      const n = noteAt(notes, f.seconds);
      if (n >= 0) folding = notes[n].midi;
      out.push({ seconds: f.seconds, midi: displayMidi(f.pitch.midi, folding, session.settings.octave), feedback: frameFeedback(f.comparison) });
    }
    return out;
  }

  // ---- Session --------------------------------------------------------------------------------

  let karaoke = $state<Karaoke>({ kind: "idle" });

  function karaokeEvent(e: KaraokeEvent): boolean {
    const next = karaokeNext(karaoke, e);
    if (!next) return false;
    karaoke = next;
    return true;
  }

  const listen = listener();

  /** Shows take `id`'s full score in the Score view. */
  async function useForScoring(id: number) {
    session.score = { kind: "scoring" };
    session.view = "Score";
    try {
      session.score = { kind: "scored", take: id, report: await host.scoreRecorded(id, settings()) };
    } catch (e) {
      session.score = { kind: "failed", message: errorMessage(e) };
    }
  }

  /** Seconds heard before singing starts. */
  const PRE_ROLL = 2;

  const seekTo = (s: number) => run([{ kind: "Seek", frame: Math.max(0, Math.min(project.frames - 1, Math.floor(s * snapshot.sample_rate))) }]);

  async function start() {
    listen({ kind: "stop" });
    const fresh = karaoke.kind !== "paused";
    if (session.mic.kind !== "free" || !karaokeEvent(fresh ? { kind: "start" } : { kind: "resume" })) return;
    session.mic = { kind: "recording" };
    // The pre-roll: start a little before where the singer will come in.
    const from = loop && fresh ? loop.start : seconds;
    await seekTo(from - PRE_ROLL);
    if (fresh) {
      performance_ = newPerformance(notes);
      trail.clear();
      trail.fills = performance_.fills;
      folding = null;
      refresh();
    }
    try {
      await armTake(host, project, effectiveBacking(session));
    } catch (e) {
      karaokeEvent({ kind: "fail", message: errorMessage(e) });
    }
  }

  // The microphone is free again whenever no run is live.
  $effect(() => {
    if (!isLive(karaoke) && session.mic.kind === "recording") session.mic = { kind: "free" };
  });

  /** Saves the run that just ended as a take; it's scored offline and shows in Score. */
  async function saveRun(): Promise<boolean> {
    try {
      const added = await finishTake(host, project, settings());
      applyTake(session, added);
      celebrate(endRun(performance_, added.take));
      refresh();
      return true;
    } catch (e) {
      endRun(performance_, null);
      refresh();
      karaokeEvent({ kind: "fail", message: errorMessage(e) });
      return false;
    }
  }

  async function pause() {
    if (!karaokeEvent({ kind: "pause" })) return;
    await run([{ kind: "Pause" }]);
    if (!(await saveRun())) return;
    const t = (await host.transport()).transport;
    karaokeEvent({ kind: "paused", frame: t.kind === "Paused" ? t.frame : 0 });
  }

  async function finish() {
    const from = karaoke.kind;
    if (!karaokeEvent({ kind: "finish" })) return;
    if (from === "paused") return;
    await run([{ kind: "Pause" }]);
    if (from === "countIn") {
      // Nothing was sung in this run: drop it.
      await cancelTake(host, project).catch((e) => console.warn(errorMessage(e)));
      endRun(performance_, null);
      karaokeEvent({ kind: "saved" });
    } else if (await saveRun()) {
      karaokeEvent({ kind: "saved" });
    }
  }

  // The transport ends the count-in, and the song's end (or Stop) finishes the performance.
  $effect(() => {
    const e = observeKaraoke(karaoke, snapshot.transport);
    if (!e) return;
    if (e.kind === "finish") void finish();
    else karaokeEvent(e);
  });

  // ---- Loop A/B: each pass is a take -----------------------------------------------------------

  const loop = $derived(session.loop);
  type LoopMark = { kind: "none" } | { kind: "a"; start: number };
  let marking = $state<LoopMark>({ kind: "none" });

  function markLoop() {
    if (loop) {
      session.loop = null;
      marking = { kind: "none" };
    } else if (marking.kind === "none") marking = { kind: "a", start: seconds };
    else {
      const [a, b] = [marking.start, seconds].sort((x, y) => x - y);
      if (b - a > 0.5) session.loop = { start: a, end: b };
      marking = { kind: "none" };
    }
  }

  // Passing B while singing saves the pass and counts in again from A.
  let passing = false;
  $effect(() => {
    if (!loop || karaoke.kind !== "singing" || seconds < loop.end || passing) return;
    passing = true;
    void (async () => {
      await pause();
      // `pause` moved the machine on; read it afresh.
      if ((karaoke as Karaoke).kind === "paused") {
        await seekTo(loop.start - PRE_ROLL);
        await start();
      }
      passing = false;
    })();
  });

  // ---- Seeking: drag the lane, click the ruler, click a lyric line -----------------------------

  type Scrub = { kind: "idle" } | { kind: "dragging"; x0: number; from: number; pointer: number };
  let scrub = $state<Scrub>({ kind: "idle" });
  /** Seconds of song across the lane (the trail's window). */
  const LANE_SECONDS = 8;

  function laneDown(e: PointerEvent) {
    if (live || e.button !== 0) return;
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    scrub = { kind: "dragging", x0: e.clientX, from: seconds, pointer: e.pointerId };
  }
  function laneMove(e: PointerEvent) {
    if (scrub.kind !== "dragging" || e.pointerId !== scrub.pointer || !lane) return;
    // Dragging left moves later in the song, like pulling the lane along.
    void seekTo(scrub.from - ((e.clientX - scrub.x0) / lane.clientWidth) * LANE_SECONDS);
  }
  const laneUp = () => (scrub = { kind: "idle" });

  function rulerClick(e: MouseEvent) {
    if (live) return;
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    void seekTo(((e.clientX - r.left) / r.width) * duration);
  }

  // Live pitch while a run is recording.
  const polling = $derived(karaoke.kind === "countIn" || karaoke.kind === "singing");
  $effect(() => {
    if (!polling || mic.kind !== "available") return;
    return eachFrame(async () => {
      try {
        const update = await mic.live(settings());
        trail.add(points(update.frames));
        celebrate(addFrames(performance_, notes, update.frames, update.totals));
        refresh();
      } catch (e) {
        console.warn(errorMessage(e));
      }
    });
  });

  // Leaving the view mid-run keeps the singing: the run is saved like a finish.
  $effect(() => () => {
    if (isLive(karaoke)) void finish();
  });

  $effect(() => {
    if (karaoke.kind === "failed") toast(`Karaoke stopped: ${karaoke.message}`, "error");
  });

  // ---- Lyrics ---------------------------------------------------------------------------------

  const lyrics = $derived(project.lyrics);
  const lines = $derived(lyrics.kind === "Synced" ? lyrics.lines : []);
  const lineIndex = $derived(Math.max(0, currentLine(lines, seconds)));
  /** The line being sung and the next; an instrumental break shows the next line early. */
  const visible = $derived(lines.slice(lineIndex, lineIndex + 2));

  const beats = $derived(beatsLeft(snapshot.transport, project.analysis.tempo.bpm, snapshot.sample_rate));
  const live = $derived(isLive(karaoke));
  const busy = $derived(live || session.mic.kind === "calibrating");
  const duration = $derived(project.frames / project.sample_rate);

  const hints = $derived([
    "Use headphones, so the backing doesn't bleed into the microphone.",
    "Drag the lane or click the ruler to move; click a lyric line to start from it. Sing starts 2 s early so you hear the lead-in.",
    "Loop: press once at A and again at B. Each pass is saved as its own take.",
    ...(backingHint(session) ? [backingHint(session)!] : []),
  ]);
</script>

<section class="karaoke">
  {#snippet loopButton()}
    <button class="press quiet small-button loop" class:loop-on={loop !== null || marking.kind === "a"} disabled={live} title={loop ? "Clear the loop" : marking.kind === "a" ? "Set B here" : "Set A here"} onclick={markLoop}>
      <Icon name="loop" />{loop ? `${formatTime(loop.start)}–${formatTime(loop.end)}` : marking.kind === "a" ? "Set B" : "Loop"}
    </button>
  {/snippet}
  {#snippet rules()}
    <SettingsControls {session} disabled={live} />
  {/snippet}
  <Toolbar {hints} items={[{ key: "loop", control: loopButton }, { key: "rules", control: rules }]}>
    <BackingMenu {session} disabled={live} />
    <TakePicker {host} {session} />
  </Toolbar>

  <div class="lane" bind:this={lane} role="slider" tabindex="-1" aria-label="Drag to move through the song" aria-valuemin={0} aria-valuemax={Math.round(duration)} aria-valuenow={Math.round(seconds)} onpointerdown={laneDown} onpointermove={laneMove} onpointerup={laneUp} onpointercancel={laneUp}>
    <Grain behaviour={trail} style="width: 100%; height: 300px" label="Pitch lane: the song's notes, and your voice as a trail" />
    <div class="hud">
      <div class="stat">
        <span class="label">Score</span>
        <strong class="pixel big-number">{shown.score.toFixed(0)}</strong>
      </div>
      <div class="stat">
        <span class="label">Streak</span>
        <strong class="value">{shown.streak}</strong>
      </div>
      {#if karaoke.kind === "countIn"}
        <strong class="pixel countdown" in:enter>{beats || ""}</strong>
      {/if}
    </div>
  </div>

  <button class="ruler" aria-label="Song position: click to move there" disabled={live} onclick={rulerClick}>
    {#if loop}<span class="loop" style:left="{(loop.start / duration) * 100}%" style:width="{((loop.end - loop.start) / duration) * 100}%"></span>{/if}
    <span class="fill" style:transform="scaleX({Math.min(1, seconds / duration)})"></span>
  </button>

  <div class="lyrics" aria-live="off">
    {#if lyrics.kind === "Synced"}
      {#each visible as line, k (line.lrc_start + ":" + k)}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
        <p class="line" class:next={k === 1} in:enter onclick={() => !live && void seekTo(line.syllables[0]?.start ?? line.lrc_start)}>
          {#if line.syllables.length === 0}
            <span class="muted">♪</span>
          {:else}
            {#each line.syllables as s, j (j)}
              <span class="syllable" data-text={s.text} style:--sung={sungFraction(s.start, s.end, seconds)}>{s.text}</span>
            {/each}
          {/if}
        </p>
      {/each}
    {:else if lyrics.kind === "Plain"}
      <div class="plain">
        {#each lyrics.lines as text, i (i)}<p class="muted">{text}</p>{/each}
      </div>
    {/if}
  </div>

  <div class="status">
    {#if karaoke.kind === "countIn"}
      <p class="muted">Count-in: sing from the next bar.</p>
    {:else if karaoke.kind === "singing"}
      <p class="muted">Pause saves what you've sung so far; Finish (or the song's end) wraps up.</p>
    {:else if karaoke.kind === "paused"}
      <p class="muted">Paused at {formatTime(karaoke.frame / snapshot.sample_rate)}. That run is saved as a take; Resume counts you back in.</p>
    {:else if karaoke.kind === "pausing" || karaoke.kind === "finishing"}
      <p class="muted">Saving your take</p>
    {:else if karaoke.kind === "finished"}
      <div class="result" in:enter>
        <p>
          You scored <strong>{shown.score.toFixed(0)}</strong> with a best streak of <strong>{shown.best}</strong>.
          {shown.takes === 1 ? "Saved as a take" : `Saved as ${shown.takes} takes`}, scored in full in the Score view.
        </p>
        <button class="press raised big" onclick={() => (session.view = "Score")}>Open in Score</button>
        <button class="press quiet big" onclick={() => karaokeEvent({ kind: "reset" })}>Done</button>
      </div>
    {:else if karaoke.kind === "failed"}
      <p class="muted">{karaoke.message}</p>
    {:else}
      <p class="muted">Sing from the playhead: move to where you want to start, then press Sing.</p>
    {/if}
  </div>

  {#if !live}
    <Takes {host} {session} onUse={useForScoring} empty="Each run you sing is saved here as a take, ready to play back." />
    <div class="setup">
      <LyricsPanel {host} {session} locked={live} />
    </div>
  {/if}

  <BottomAction>
    {#if karaoke.kind === "singing" || karaoke.kind === "pausing"}
      <button class="press cta" onclick={pause} disabled={karaoke.kind === "pausing"}><Icon name="pause" />Pause</button>
    {:else}
      <button class="press cta sing" onclick={start} disabled={busy || mic.kind !== "available" || karaoke.kind === "finishing"} title={mic.kind === "unavailable" ? mic.reason : undefined}>
        <Icon name="record" />{karaoke.kind === "paused" ? "Resume" : "Sing"}
      </button>
    {/if}
    {#snippet side()}
      {#if live || karaoke.kind === "paused"}
        <button class="press raised big" onclick={finish} disabled={karaoke.kind === "pausing" || karaoke.kind === "finishing"}>Finish</button>
      {/if}
    {/snippet}
  </BottomAction>
</section>

<style>
  .ruler {
    position: relative;
    height: 16px;
    border: 0;
    padding: 0;
    background: var(--c-surface);
    border-radius: 8px;
    overflow: hidden;
    cursor: pointer;
  }
  .ruler .fill {
    position: absolute;
    inset: 0;
    background: var(--c-raised);
    transform-origin: left;
  }
  .ruler .loop {
    position: absolute;
    top: 0;
    bottom: 0;
    background: var(--c-near);
    opacity: 0.35;
    z-index: 1;
  }
  .loop {
    display: flex;
    align-items: center;
    gap: 8px;
    white-space: nowrap;
  }
  .loop-on {
    color: var(--c-near);
  }
  .line {
    cursor: pointer;
  }
  .lane {
    touch-action: none;
    cursor: grab;
  }
  :global(.cta.sing) {
    gap: 8px;
  }
  .karaoke {
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  .lane {
    position: relative;
    background: var(--c-surface);
    border-radius: 16px;
    overflow: hidden;
  }
  .hud {
    position: absolute;
    top: 16px;
    left: 20px;
    right: 20px;
    display: flex;
    align-items: flex-start;
    gap: 32px;
    pointer-events: none;
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .big-number {
    font-size: 40px;
    line-height: 44px;
    font-weight: 400;
    font-variant-numeric: tabular-nums;
  }
  .value {
    font-size: 20px;
    line-height: 28px;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
  }
  .countdown {
    margin-left: auto;
    font-size: 64px;
    line-height: 64px;
    font-weight: 400;
  }
  .label {
    font-size: 13px;
    line-height: 18px;
    font-weight: 500;
    color: var(--c-muted);
  }
  .lyrics {
    min-height: 96px;
    display: grid;
    gap: 8px;
  }
  .line {
    margin: 0;
    font-size: 28px;
    line-height: 40px;
    font-weight: 500;
  }
  .line.next {
    font-size: 20px;
    line-height: 28px;
    opacity: 0.6;
  }
  /* The karaoke wipe: a sung copy of each syllable, revealed left to right as it's sung. */
  .syllable {
    position: relative;
    color: var(--c-muted);
    white-space: pre;
  }
  .syllable::after {
    content: attr(data-text);
    position: absolute;
    left: 0;
    top: 0;
    color: var(--c-text);
    white-space: pre;
    clip-path: inset(0 calc((1 - var(--sung)) * 100%) 0 0);
  }
  .plain {
    max-height: 240px;
    overflow-y: auto;
  }
  .plain p,
  .status p {
    margin: 0;
  }
  .result {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
  }
  .setup {
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
</style>
