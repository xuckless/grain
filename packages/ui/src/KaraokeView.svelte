<script lang="ts">
  import type { EngineHost, LiveFrame, PlayerCommand, TransportSnapshot } from "@workstation/engine-api";
  import Grain from "./Grain.svelte";
  import LyricsPanel from "./LyricsPanel.svelte";
  import RecordingSetup from "./RecordingSetup.svelte";
  import SettingsControls from "./SettingsControls.svelte";
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
  import { applyTake, armTake, cancelTake, eachFrame, effectiveBacking, finishTake, runCommands } from "./recorder";
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
  let calibrating = $state(false);

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

  async function start() {
    listen({ kind: "stop" });
    const fresh = karaoke.kind !== "paused";
    if (!karaokeEvent(fresh ? { kind: "start" } : { kind: "resume" })) return;
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
  const busy = $derived(live || calibrating);
</script>

<section class="karaoke">
  <div class="controls">
    <SettingsControls {session} disabled={live} />
    <span class="spacer"></span>
    {#if karaoke.kind === "singing" || karaoke.kind === "pausing"}
      <button class="press raised big" onclick={pause} disabled={karaoke.kind === "pausing"}>Pause</button>
    {:else if !live}
      <button
        class="press primary big"
        onclick={start}
        disabled={busy || mic.kind !== "available"}
        title={mic.kind === "unavailable" ? mic.reason : undefined}
      >
        {karaoke.kind === "paused" ? "Resume" : "Sing"}
      </button>
    {/if}
    {#if live || karaoke.kind === "paused"}
      <button class="press raised big" onclick={finish} disabled={karaoke.kind === "pausing" || karaoke.kind === "finishing"}>Finish</button>
    {/if}
  </div>

  <div class="lane" bind:this={lane}>
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

  <div class="lyrics" aria-live="off">
    {#if lyrics.kind === "Synced"}
      {#each visible as line, k (line.lrc_start + ":" + k)}
        <p class="line" class:next={k === 1} in:enter>
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
      <p class="muted">Sing from the playhead: seek where you want to start, then press Sing. Use headphones, so the backing doesn't bleed into the microphone.</p>
    {/if}
  </div>

  {#if !live}
    <Takes {host} {session} onUse={useForScoring} empty="Each run you sing is saved here as a take, ready to play back." />
    <div class="setup">
      <RecordingSetup {host} {session} busy={live} bind:calibrating beforeCalibrate={async () => listen({ kind: "stop" })} />
      <LyricsPanel {host} {session} locked={live} />
    </div>
  {/if}
</section>

<style>
  .karaoke {
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  .controls {
    display: flex;
    align-items: center;
    gap: 24px;
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
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
