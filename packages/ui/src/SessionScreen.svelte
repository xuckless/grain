<script lang="ts">
  import type { EngineHost, MixParams, PlayerCommand, Project, TransportSnapshot } from "@workstation/engine-api";
  import AutoTuneView from "./AutoTuneView.svelte";
  import ExportDialog from "./ExportDialog.svelte";
  import SettingsPanel from "./SettingsPanel.svelte";
  import { commitChain, takeById } from "./chain";
  import { describe, redo, stateFor, undo, type Command, type Direction } from "./history";
  import PitchView from "./PitchView.svelte";
  import ProduceView from "./ProduceView.svelte";
  import Grain from "./Grain.svelte";
  import Icon from "./Icon.svelte";
  import KaraokeView from "./KaraokeView.svelte";
  import MixView from "./MixView.svelte";
  import ScoreView from "./ScoreView.svelte";
  import Tabs from "./Tabs.svelte";
  import TopBar from "./TopBar.svelte";
  import { provideListen, services } from "./context";
  import { listenCommands, listenNext, observeListening, type ListenEvent } from "./listen";
  import { runCommands } from "./recorder";
  import { Orb } from "./grain";
  import { enter, iconSwap, popover, pulse, receiveShared, sendShared } from "./motion";
  import { clockText } from "./clock";
  import { formatTime, songTitle, viewsFor, type Session, type ViewId } from "./state";

  let {
    host,
    session,
    snapshot,
    onBack,
    onClose,
    onSave,
    onCommand,
    onMix,
    onView,
    onModels,
  }: {
    host: EngineHost;
    session: Session;
    snapshot: TransportSnapshot;
    onBack: () => void;
    onClose: () => void;
    onSave: () => Promise<void>;
    onCommand: (c: PlayerCommand) => void;
    onMix: (track: number, params: MixParams) => void;
    onView: (id: ViewId) => void;
    onModels: () => void;
  } = $props();

  const { burst, toast } = services();
  const mark = new Orb(2.4);

  type CloseState = { kind: "idle" } | { kind: "confirming" };
  let closing = $state<CloseState>({ kind: "idle" });
  /** The top bar's panels: one open at a time. */
  type Panel = { kind: "none" } | { kind: "settings" } | { kind: "export" };
  let panel = $state<Panel>({ kind: "none" });
  const togglePanel = (kind: "settings" | "export") => (panel = panel.kind === kind ? { kind: "none" } : { kind });
  let saveButton = $state<HTMLButtonElement | undefined>();
  let saves = $state(0);
  let closeRoot = $state<HTMLDivElement | undefined>();

  const project = $derived(session.project);
  const title = $derived(songTitle(project));
  const duration = $derived(project.frames / project.sample_rate);

  /** The engine changed the project (a MIDI layer was added): show it; it's unsaved. */
  function adopt(next: Project) {
    session.adopt(next);
  }
  const playing = $derived(snapshot.transport.kind === "Playing");

  // ---- Listening to a take (one playback at a time, listen.ts) --------------------------------

  /** Applies a listen event; the player follows. Rejected events change nothing. */
  function listen(e: ListenEvent) {
    const prev = session.listening;
    const next = listenNext(prev, e);
    if (!next) return;
    session.listening = next;
    void runCommands(host, listenCommands(session.project, prev, next, snapshot.sample_rate));
  }
  provideListen(listen);

  // The transport starts and ends each item (and a pause elsewhere ends it).
  $effect(() => {
    const e = observeListening(session.project, session.listening, snapshot.transport, snapshot.sample_rate);
    if (e) listen(e);
  });

  /** The top bar's play/pause: the song, or the take being heard. Playing the song ends any
   * listening (its mix goes back first). */
  function playPause() {
    const l = session.listening;
    if (l.kind === "playing") listen({ kind: "pause" });
    else if (l.kind === "paused") listen({ kind: "resume" });
    else onCommand(playing ? { kind: "Pause" } : { kind: "Play" });
  }

  function stop() {
    listen({ kind: "stop" });
    onCommand({ kind: "Stop" });
  }

  // Leaving the song (or Back to Home) puts the mix back.
  $effect(() => () => listen({ kind: "stop" }));
  const key = $derived(project.analysis.keys[0]);
  const canSave = $derived(host.projects.kind === "available");
  const unsaved = $derived(canSave && session.save.kind === "dirty");

  async function save() {
    if (!canSave || session.save.kind === "saving") return;
    await onSave();
    if (session.save.kind === "clean") {
      saves += 1; // pulse (rule 8)
      if (saveButton) burst.fireAt(saveButton);
    }
  }

  function requestClose() {
    if (unsaved) closing = { kind: "confirming" };
    else onClose();
  }

  async function saveAndClose() {
    await save();
    if (session.save.kind === "clean") onClose();
  }

  // ---- Undo and redo (history.ts) -------------------------------------------------------------

  type Travel = { kind: "idle" } | { kind: "applying" };
  let travel = $state<Travel>({ kind: "idle" });

  /** Sets what `c` changed to its state in `direction` (without recording it again). */
  async function applyCommand(c: Command, direction: Direction): Promise<void> {
    switch (c.kind) {
      case "Mix":
        return onMix(c.track, stateFor(c, direction));
      case "Master":
        return session.saved(await host.takes.setMaster(stateFor(c, direction)));
      case "Rename":
        return session.saved(await host.takes.rename(c.take, stateFor(c, direction)));
      case "PitchEdit": {
        await host.pitchEditor.set(stateFor(c, direction));
        session.pitchRevision += 1;
        session.touch();
        const id = c.source.kind === "Take" ? c.source.id : null;
        const take = takeById(session.project, id);
        // An applied pitch step re-renders with the restored edits.
        if (take && take.chain.pitch.state === "Active") await commitChain(host, session, session.project, take.id, take.chain, "pitch", false);
        return;
      }
      case "Chain": {
        const job = await commitChain(host, session, session.project, c.take, stateFor(c, direction), "pitch", false);
        if (job.kind === "failed") throw new Error(job.message);
        if (job.kind === "needsModel") throw new Error(`needs the ${job.model} model`);
        return;
      }
    }
  }

  async function step(direction: Direction) {
    if (travel.kind === "applying") return;
    const moved = direction === "undo" ? undo(session.history) : redo(session.history);
    if (!moved) return;
    travel = { kind: "applying" };
    try {
      await applyCommand(moved.command, direction);
      session.history = moved.history;
    } catch (e) {
      toast(`Couldn't ${direction} the ${describe(moved.command)}: ${e instanceof Error ? e.message : String(e)}`, "error");
    } finally {
      travel = { kind: "idle" };
    }
  }

  const lastDone = $derived(session.history.past.at(-1));
  const nextUndone = $derived(session.history.future[0]);

  // ⌘S / Ctrl+S saves; ⌘Z / ⇧⌘Z undo and redo; Space plays and pauses (unless a control has
  // focus).
  $effect(() => {
    const keys = (e: KeyboardEvent) => {
      const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void save();
      } else if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "z" && !typing) {
        e.preventDefault();
        void step(e.shiftKey ? "redo" : "undo");
      } else if (e.key === " " && !(e.target instanceof HTMLButtonElement || e.target instanceof HTMLInputElement)) {
        e.preventDefault();
        playPause();
      } else if (e.key === "Escape" && closing.kind === "confirming") {
        closing = { kind: "idle" };
      }
    };
    window.addEventListener("keydown", keys);
    return () => window.removeEventListener("keydown", keys);
  });

  // A press outside the confirm popover dismisses it.
  $effect(() => {
    if (closing.kind !== "confirming") return;
    const away = (e: PointerEvent) => {
      if (closeRoot && !closeRoot.contains(e.target as Node)) closing = { kind: "idle" };
    };
    window.addEventListener("pointerdown", away);
    return () => window.removeEventListener("pointerdown", away);
  });
</script>

<div class="screen">
  <TopBar chrome={host.chrome}>
    {#snippet left()}
      <button class="press icon quiet" aria-label="Back to home" onclick={onBack}><Icon name="back" /></button>
      <Grain behaviour={mark} style="width: 28px; height: 28px; flex: none" />
      <span class="song" in:receiveShared|global={{ key: `title:${title}` }} out:sendShared|global={{ key: `title:${title}` }}>{title}</span>
    {/snippet}
    {#snippet center()}
      <button class="press play" aria-label={playing ? "Pause" : "Play"} onclick={playPause}>
        {#key playing}<span class="glyph" transition:iconSwap><Icon name={playing ? "pause" : "play"} /></span>{/key}
      </button>
      <button class="press icon quiet" aria-label="Stop" disabled={snapshot.transport.kind === "Stopped"} onclick={stop}><Icon name="stop" /></button>
      <span class="mono readout"><span use:clockText={formatTime}></span><span class="muted total">/ {formatTime(duration)}</span></span>
      <span class="mono muted fact bpm">{project.analysis.tempo.bpm.toFixed(0)} BPM</span>
      {#if key}<span class="mono muted fact key">{key.label}</span>{/if}
    {/snippet}
    {#snippet right()}
      <button class="press icon quiet" aria-label="Undo" title={lastDone ? `Undo ${describe(lastDone)} (⌘Z)` : "Nothing to undo"} disabled={!lastDone || travel.kind === "applying"} onclick={() => step("undo")}><Icon name="undo" /></button>
      <button class="press icon quiet redo" aria-label="Redo" title={nextUndone ? `Redo ${describe(nextUndone)} (⇧⌘Z)` : "Nothing to redo"} disabled={!nextUndone || travel.kind === "applying"} onclick={() => step("redo")}><Icon name="redo" /></button>
      <button class="press quiet save" title="Download and manage the AI models" onclick={onModels}>Models</button>
      <div class="anchor">
        <button class="press quiet save" data-export-trigger aria-expanded={panel.kind === "export"} title="Export a mixdown" onclick={() => togglePanel("export")}><Icon name="export" size={16} />Export…</button>
        {#if panel.kind === "export"}
          <ExportDialog {host} {session} loop={session.loop} onClose={() => (panel = { kind: "none" })} />
        {/if}
      </div>
      <div class="anchor">
        <button class="press icon quiet" data-settings-trigger aria-label="Settings" aria-expanded={panel.kind === "settings"} onclick={() => togglePanel("settings")}><Icon name="gear" /></button>
        {#if panel.kind === "settings"}
          <SettingsPanel {host} {session} onClose={() => (panel = { kind: "none" })} beforeCalibrate={() => listen({ kind: "stop" })} />
        {/if}
      </div>
      <button
        class="press save"
        class:primary={session.save.kind !== "clean" && canSave}
        bind:this={saveButton}
        use:pulse={saves}
        disabled={!canSave || session.save.kind === "saving"}
        title={host.projects.kind === "unavailable" ? host.projects.reason : "Save (⌘S)"}
        onclick={save}
      >
        {#if session.save.kind === "clean" && canSave}<Icon name="check" size={16} />{/if}
        {#key session.save.kind}<span class="swap">{session.save.kind === "clean" && canSave ? "Saved" : session.save.kind === "saving" ? "Saving" : "Save"}</span>{/key}
      </button>
      <div class="close" bind:this={closeRoot}>
        <button class="press icon quiet" aria-label="Close song" aria-expanded={closing.kind === "confirming"} onclick={requestClose}><Icon name="close" /></button>
        {#if closing.kind === "confirming"}
          <div class="confirm" role="dialog" aria-label="Close without saving?" transition:popover>
            <strong>Close without saving?</strong>
            <span class="muted">Mix changes since your last save will be lost.</span>
            <div class="actions">
              <button class="press quiet" onclick={() => (closing = { kind: "idle" })}>Cancel</button>
              <button class="press raised" onclick={onClose}>Discard</button>
              <button class="press primary" onclick={saveAndClose}>Save and close</button>
            </div>
          </div>
        {/if}
      </div>
    {/snippet}
  </TopBar>

  <div class="body">
    <Tabs views={viewsFor(project)} view={session.view} onSelect={onView} />
    <div class="stage">
      {#key session.view}
        <div class="layer" transition:enter>
          {#if session.view === "Karaoke"}
            <KaraokeView {host} {session} {snapshot} />
          {:else if session.view === "AutoTune"}
            <AutoTuneView {host} {session} onSeek={(frame) => onCommand({ kind: "Seek", frame })} />
          {:else if session.view === "Pitch"}
            <PitchView {host} {session} {snapshot} onSeek={(frame) => onCommand({ kind: "Seek", frame })} />
          {:else if session.view === "Produce"}
            <ProduceView {host} {session} {onMix} {onModels} />
          {:else if session.view === "Score"}
            <ScoreView {host} {session} {snapshot} onSeek={(frame) => onCommand({ kind: "Seek", frame })} />
          {:else}
            <MixView
              {project}
              {snapshot}
              transcriber={host.transcriber}
              view={session.viewport}
              onView={(v) => (session.viewport = v)}
              onSeek={(frame) => onCommand({ kind: "Seek", frame })}
              {onMix}
              onMixCommit={(track, before, after) => session.record({ kind: "Mix", track, before, after })}
              onProject={adopt}
              {onModels}
            />
          {/if}
        </div>
      {/key}
    </div>
  </div>
</div>

<style>
  .screen {
    height: 100vh;
    display: flex;
    flex-direction: column;
  }
  .song {
    font-size: 15px;
    line-height: 22px;
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    margin-left: 4px;
  }
  .play {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    background: var(--c-text);
    color: var(--c-ground);
    display: grid;
    place-items: center;
    padding: 0;
    flex: none;
  }
  .glyph {
    grid-area: 1 / 1;
    display: flex;
  }
  .readout,
  .fact {
    white-space: nowrap;
  }
  .readout {
    margin-left: 8px;
  }
  .total {
    margin-left: 1ch;
  }
  .fact {
    margin-left: 4px;
  }
  .redo {
    margin-right: 8px; /* edit history, then the song's own actions */
  }
  /* When the bar runs short of room, the song facts give way first, in this order (the window
     buttons and every action keep their place): the key, the tempo, then the total time. Each
     threshold leaves the title about 100 px beside the transport (362 px with everything) and
     the actions (544 px). */
  @container topbar (max-width: 1140px) {
    .key {
      display: none;
    }
  }
  @container topbar (max-width: 1060px) {
    .bpm {
      display: none;
    }
  }
  @container topbar (max-width: 1000px) {
    .total {
      display: none;
    }
  }
  .save {
    height: 44px;
    padding: 0 18px;
    border-radius: 12px;
    background: var(--c-raised);
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 96px;
    justify-content: center;
  }
  .close,
  .anchor {
    position: relative;
  }
  .confirm {
    position: absolute;
    right: 0;
    top: 52px;
    width: 320px;
    box-sizing: border-box;
    padding: 16px 18px;
    background: var(--c-raised);
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 14px;
    line-height: 21px;
    transform-origin: top right; /* rules 11–12: from and back to the close button */
    z-index: 40;
  }
  .confirm strong {
    font-weight: 500;
    font-size: 15px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 12px;
  }
  .actions button {
    height: 40px;
    padding: 0 14px;
    border-radius: 10px;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    box-sizing: border-box;
    padding: 8px 24px 32px;
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  .stage {
    display: grid;
  }
  .layer {
    grid-area: 1 / 1;
    min-width: 0;
  }
</style>
