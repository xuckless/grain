<script lang="ts">
  import type { EngineHost, MixParams, PlayerCommand, Project, TransportSnapshot } from "@workstation/engine-api";
  import AutoTuneView from "./AutoTuneView.svelte";
  import PitchView from "./PitchView.svelte";
  import ProduceView from "./ProduceView.svelte";
  import Grain from "./Grain.svelte";
  import Icon from "./Icon.svelte";
  import KaraokeView from "./KaraokeView.svelte";
  import MixView from "./MixView.svelte";
  import ScoreView from "./ScoreView.svelte";
  import Tabs from "./Tabs.svelte";
  import TopBar from "./TopBar.svelte";
  import { services } from "./context";
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

  const { burst } = services();
  const mark = new Orb(2.4);

  type CloseState = { kind: "idle" } | { kind: "confirming" };
  let closing = $state<CloseState>({ kind: "idle" });
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

  // ⌘S / Ctrl+S saves; Space plays and pauses (unless a control has focus).
  $effect(() => {
    const keys = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
        e.preventDefault();
        void save();
      } else if (e.key === " " && !(e.target instanceof HTMLButtonElement || e.target instanceof HTMLInputElement)) {
        e.preventDefault();
        onCommand(playing ? { kind: "Pause" } : { kind: "Play" });
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
      <button class="press play" aria-label={playing ? "Pause" : "Play"} onclick={() => onCommand(playing ? { kind: "Pause" } : { kind: "Play" })}>
        {#key playing}<span class="glyph" transition:iconSwap><Icon name={playing ? "pause" : "play"} /></span>{/key}
      </button>
      <button class="press icon quiet" aria-label="Stop" disabled={snapshot.transport.kind === "Stopped"} onclick={() => onCommand({ kind: "Stop" })}><Icon name="stop" /></button>
      <span class="mono readout"><span use:clockText={formatTime}></span> <span class="muted">/ {formatTime(duration)}</span></span>
      <span class="mono muted">{project.analysis.tempo.bpm.toFixed(0)} BPM</span>
      {#if key}<span class="mono muted">{key.label}</span>{/if}
    {/snippet}
    {#snippet right()}
      <button class="press quiet save" title="Download and manage the AI models" onclick={onModels}>Models</button>
      {#if host.projects.kind === "available" && host.projects.archive.kind === "available"}
        {@const archive = host.projects.archive}
        <button class="press quiet save" title="Download this song, its stems and takes as one .grain file" onclick={() => archive.exportSong()}>Export</button>
      {/if}
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
            <MixView {project} {snapshot} transcriber={host.transcriber} onSeek={(frame) => onCommand({ kind: "Seek", frame })} {onMix} onProject={adopt} {onModels} />
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
  .readout {
    margin-left: 8px;
    white-space: nowrap;
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
  .close {
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
