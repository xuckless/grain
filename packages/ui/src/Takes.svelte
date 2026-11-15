<script lang="ts">
  import type { EngineHost, Take } from "@workstation/engine-api";
  import type { Action } from "svelte/action";
  import Confirm from "./Confirm.svelte";
  import Icon from "./Icon.svelte";
  import { clock, onFrame } from "./clock";
  import { listener, services } from "./context";
  import { dotRow } from "./grain";
  import { isListening, listenProgress, madeFrom, sungTakes, LISTEN_MODES, type ListenItem } from "./listen";
  import { enter } from "./motion";
  import { errorMessage, type Session } from "./state";
  import { deleteTake } from "./takeops";

  /**
   * The song's takes, newest first, each playable on its own or over the backing (the one
   * listen machine, listen.ts), renameable and deletable. A take made from another one (tuned,
   * edited, cleaned, chained) can be compared with it: the original, then this one. Every
   * change here is saved at once (takes autosave, AGENTS.md §11).
   */
  let {
    host,
    session,
    onUse,
    empty = "No takes yet.",
  }: {
    host: EngineHost;
    session: Session;
    /** "Use for scoring" (Score and Karaoke): shows this take's score. */
    onUse?: (id: number) => void;
    empty?: string;
  } = $props();

  const listen = listener();
  const { toast } = services();

  const project = $derived(session.project);
  const takes = $derived(sungTakes(project).slice().reverse());
  const mode = $derived(session.playback);

  /** One row at a time is being renamed, asked about deleting, or saving. */
  type RowEdit =
    | { kind: "idle" }
    | { kind: "renaming"; id: number; draft: string }
    | { kind: "confirmingDelete"; id: number }
    | { kind: "saving"; id: number };
  let edit = $state<RowEdit>({ kind: "idle" });

  const item = (id: number): ListenItem => ({ kind: "Take", id });
  const playingTake = (id: number) => session.listening.kind === "playing" && isListening(session.listening, item(id));

  function toggle(t: Take) {
    if (playingTake(t.id)) listen({ kind: "pause" });
    else if (session.listening.kind === "paused" && isListening(session.listening, item(t.id))) listen({ kind: "resume" });
    else listen({ kind: "play", items: [item(t.id)], mode });
  }

  function compare(t: Take, from: ListenItem) {
    listen({ kind: "play", items: [from, item(t.id)], mode });
  }

  const nameOf = (from: ListenItem) => (from.kind === "VocalStem" ? "the vocal stem" : `“${project.takes.find((t) => t.id === from.id)?.name ?? "the original"}”`);

  async function rename(id: number, draft: string) {
    const before = project.takes.find((t) => t.id === id)?.name ?? draft;
    edit = { kind: "saving", id };
    try {
      session.saved(await host.takes.rename(id, draft));
      session.record({ kind: "Rename", take: id, before, after: draft.trim() || before });
    } catch (e) {
      toast(`Couldn't rename the take: ${errorMessage(e)}`, "error");
    }
    edit = { kind: "idle" };
  }

  async function remove(id: number) {
    edit = { kind: "saving", id };
    if (session.listening.kind !== "idle" && isListening(session.listening, item(id))) listen({ kind: "stop" });
    try {
      await deleteTake(host.takes, session, id);
    } catch (e) {
      toast(`Couldn't delete the take: ${errorMessage(e)}`, "error");
    }
    edit = { kind: "idle" };
  }

  /** The playing row's progress, as dots, drawn per frame from the clock while it's mounted. */
  const progress: Action<HTMLCanvasElement> = (canvas) => {
    let last = -1;
    const stop = onFrame((now) => {
      const l = session.listening;
      if (l.kind === "idle") return;
      const f = Math.round(listenProgress(session.project, l, clock.seconds(now)) * 100) / 100;
      if (f !== last) dotRow(canvas, (last = f));
    });
    return { destroy: stop };
  };

  const focus: Action<HTMLInputElement> = (node) => {
    node.focus();
    node.select();
  };
</script>

<section class="takes" aria-label="Your takes">
  <header>
    <h3>Your takes</h3>
    <div class="modes" role="radiogroup" aria-label="Play takes">
      {#each LISTEN_MODES as m (m.id)}
        <button class="press mode" class:on={mode === m.id} role="radio" aria-checked={mode === m.id} onclick={() => (session.playback = m.id)}>{m.label}</button>
      {/each}
    </div>
  </header>
  {#if takes.length === 0}
    <p class="muted">{empty}</p>
  {:else}
    <ul>
      {#each takes as t (t.id)}
        {@const from = madeFrom(t)}
        {@const active = session.listening.kind !== "idle" && isListening(session.listening, item(t.id))}
        {@const playing = playingTake(t.id)}
        <li in:enter>
          <button class="press icon play" class:on={playing} aria-label={playing ? `Pause ${t.name}` : `Play ${t.name}`} onclick={() => toggle(t)}>
            <Icon name={playing ? "pause" : "play"} size={16} />
          </button>
          <div class="name">
            {#if edit.kind === "renaming" && edit.id === t.id}
              {@const e = edit}
              <input
                class="field small"
                aria-label="Take name"
                bind:value={e.draft}
                use:focus
                onkeydown={(k) => {
                  if (k.key === "Enter") void rename(t.id, e.draft);
                  else if (k.key === "Escape") edit = { kind: "idle" };
                }}
                onblur={() => edit.kind === "renaming" && void rename(t.id, e.draft)}
              />
            {:else}
              <span class="title">{t.name}{#if edit.kind === "saving" && edit.id === t.id}<span class="muted"> · saving</span>{/if}</span>
            {/if}
            {#if active}
              <canvas class="progress" use:progress aria-hidden="true"></canvas>
            {/if}
          </div>
          <div class="actions">
            {#if from}
              <button class="press quiet small-button" title="Play {nameOf(from)}, then this take" onclick={() => compare(t, from)}>Compare</button>
            {/if}
            {#if onUse}
              <button class="press quiet small-button" title="Show this take's score" onclick={() => onUse(t.id)}>Use for scoring</button>
            {/if}
            <button class="press quiet small-button" disabled={edit.kind === "saving"} onclick={() => (edit = { kind: "renaming", id: t.id, draft: t.name })}>Rename</button>
            <div class="delete">
              <button class="press quiet small-button" disabled={edit.kind === "saving"} aria-expanded={edit.kind === "confirmingDelete" && edit.id === t.id} onclick={() => (edit = { kind: "confirmingDelete", id: t.id })}>Delete</button>
              {#if edit.kind === "confirmingDelete" && edit.id === t.id}
                <Confirm
                  title={`Delete “${t.name}”?`}
                  body="It leaves the song. ⌘Z brings it back until you close the song."
                  confirm="Delete"
                  onCancel={() => (edit = { kind: "idle" })}
                  onConfirm={() => void remove(t.id)}
                />
              {/if}
            </div>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .takes {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 16px;
  }
  h3 {
    margin: 0;
    font-size: 15px;
    line-height: 22px;
    font-weight: 500;
    flex: 1;
  }
  .modes {
    display: flex;
    background: var(--c-surface);
    border-radius: 12px;
    padding: 2px;
  }
  .mode {
    height: 36px;
    padding: 0 14px;
    border-radius: 10px;
    color: var(--c-muted);
  }
  .mode.on {
    background: var(--c-raised);
    color: var(--c-text);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  li {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 4px 8px 4px 4px;
    border-radius: 12px;
    background: var(--c-surface);
  }
  .play {
    background: var(--c-raised);
    border-radius: 50%;
  }
  .play.on {
    background: var(--c-text);
    color: var(--c-ground);
  }
  .name {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .title {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .name .field {
    height: 36px;
  }
  .progress {
    width: 160px;
    height: 8px;
    display: block;
  }
  .actions {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .delete {
    position: relative;
  }
</style>
