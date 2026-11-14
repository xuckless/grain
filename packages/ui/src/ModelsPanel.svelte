<script lang="ts">
  import type { EngineHost, ModelEntry, ModelInfo } from "@workstation/engine-api";
  import { dot, fit } from "./grain";
  import { enter, popover } from "./motion";
  import { formatBytes, modelAction, modelSize, progress, stateLabel, withState, type Consent } from "./models";
  import type { Loading } from "./pitch";
  import { errorMessage } from "./state";
  import { colour } from "./theme";

  /**
   * The model manager (AGENTS.md §10): every model in the manifest with its size, licence and
   * state, downloaded and verified on request into the host's model store (replacing the manual
   * "copy the weights" step). Restricted (opt-in) models ask for the licence to be accepted first.
   */
  let { host, onClose }: { host: EngineHost; onClose: () => void } = $props();

  // The host is fixed for the app's lifetime.
  // svelte-ignore state_referenced_locally
  const models = host.models;

  let list = $state<Loading<ModelEntry[]>>({ kind: "loading" });
  let consent = $state<Consent>({ kind: "none" });

  async function refresh() {
    try {
      list = { kind: "ready", value: await models.list() };
    } catch (e) {
      list = { kind: "failed", message: errorMessage(e) };
    }
  }
  void refresh();

  function setState(id: string, state: ModelEntry["state"]) {
    if (list.kind === "ready") list = { kind: "ready", value: withState(list.value, id, state) };
  }

  async function download(info: ModelInfo, acceptLicense: boolean) {
    consent = { kind: "none" };
    try {
      await models.download(info.id, acceptLicense, (s) => setState(info.id, s));
    } catch (e) {
      setState(info.id, { kind: "Failed", message: errorMessage(e) });
      return;
    }
    await refresh();
  }

  function ask(info: ModelInfo) {
    if (info.distribution === "opt-in") consent = { kind: "asking", info };
    else void download(info, false);
  }

  async function remove(info: ModelInfo) {
    try {
      await models.delete(info.id);
    } catch (e) {
      setState(info.id, { kind: "Failed", message: errorMessage(e) });
      return;
    }
    await refresh();
  }

  /** A dot bar lit up to `fraction`. */
  function bar(canvas: HTMLCanvasElement, fraction: number) {
    const draw = (f: number) => {
      const w = canvas.clientWidth, h = 8;
      const c = fit(canvas, w, h);
      if (!c) return;
      for (let x = 3; x <= w - 3; x += 6) dot(c, x, h / 2, 1.5, x / w <= f ? colour.text : colour.raised);
    };
    draw(fraction);
    return { update: draw };
  }

  function keydown(e: KeyboardEvent) {
    if (e.key !== "Escape") return;
    if (consent.kind === "asking") consent = { kind: "none" };
    else onClose();
  }
</script>

<svelte:window onkeydown={keydown} />

<div class="backdrop" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && onClose()}>
  <div class="panel" role="dialog" aria-modal="true" aria-label="Models" transition:popover>
    <header>
      <div>
        <h2>Models</h2>
        <p class="muted">Downloaded from Hugging Face or GitHub into {models.location}, and checked against their published checksums.</p>
      </div>
      <button class="press quiet" onclick={onClose}>Done</button>
    </header>

    {#if list.kind === "loading"}
      <p class="muted">Looking for installed models</p>
    {:else if list.kind === "failed"}
      <p class="muted">Couldn't list the models: {list.message}</p>
    {:else}
      <ul>
        {#each list.value as entry, i (entry.info.id)}
          {@const m = entry.info}
          {@const action = modelAction(entry)}
          {@const p = progress(entry.state)}
          <li in:enter={{ detail: true, delay: i * 30 }}>
            <div class="head">
              <span class="title mono">{m.id}</span>
              <span class="muted small">{formatBytes(modelSize(m))}</span>
              <span class="badge" class:restricted={m.distribution === "opt-in"}>{m.license}{m.distribution === "opt-in" ? " · opt-in" : ""}</span>
              <span class="spacer"></span>
              <span class="mono small" class:muted={entry.state.kind !== "Ready"}>{stateLabel(entry.state)}</span>
              {#if action === "download"}
                <button class="press primary" onclick={() => ask(m)}>Download</button>
              {:else if action === "retry"}
                <button class="press" onclick={() => ask(m)}>Retry</button>
              {:else if action === "delete"}
                <button class="press quiet" onclick={() => remove(m)}>Delete</button>
              {:else if action === "installed"}
                <span class="muted small" title="These weights are outside the app's model store (a development checkout)">installed outside the app</span>
              {/if}
            </div>
            <p class="muted small">{m.task}</p>
            {#if p !== null}
              <canvas class="bar" use:bar={p}></canvas>
            {/if}
            {#if entry.state.kind === "Failed"}
              <p class="small">{entry.state.message}</p>
            {/if}
            {#if action === "manual" && m.obtain.kind === "Manual"}
              <p class="muted small">{m.obtain.note}</p>
            {:else if action === "build" && m.obtain.kind === "Build"}
              <p class="muted small">Exported from its checkpoint on a computer with PyTorch: <span class="mono">{m.obtain.command}</span></p>
            {/if}
            <p class="muted small note">{m.license_note} <a href={m.source} target="_blank" rel="noreferrer">Source</a></p>
          </li>
        {/each}
      </ul>
    {/if}

    {#if consent.kind === "asking"}
      {@const m = consent.info}
      <div class="consent" role="alertdialog" aria-label="Licence" transition:popover>
        <strong>{m.id} is licensed {m.license}</strong>
        <p class="muted">{m.license_note}</p>
        <p class="muted">It isn't part of the app's own licence and may only be used as that licence allows (non-commercial licences forbid commercial use). Download it only if you accept those terms.</p>
        <div class="actions">
          <button class="press quiet" onclick={() => (consent = { kind: "none" })}>Cancel</button>
          <button class="press primary" onclick={() => download(m, true)}>Accept and download</button>
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: color-mix(in srgb, var(--c-ground) 70%, transparent);
    display: grid;
    place-items: center;
    z-index: 20;
    padding: 16px;
  }
  .panel {
    position: relative;
    width: min(760px, 100%);
    max-height: calc(100vh - 64px);
    overflow-y: auto;
    background: var(--c-surface);
    border-radius: 16px;
    padding: 24px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    box-sizing: border-box;
  }
  header {
    display: flex;
    gap: 16px;
    align-items: flex-start;
    justify-content: space-between;
  }
  h2 {
    margin: 0 0 4px;
    font-size: 20px;
    line-height: 28px;
    font-weight: 500;
  }
  p {
    margin: 0;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  li {
    background: var(--c-raised);
    border-radius: 12px;
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .title {
    font-size: 15px;
  }
  .spacer {
    flex: 1;
  }
  .small {
    font-size: 13px;
    line-height: 18px;
  }
  .note a {
    color: var(--c-text);
  }
  .badge {
    font-size: 12px;
    line-height: 16px;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--c-disabled);
    color: var(--c-muted);
  }
  .badge.restricted {
    border-color: var(--c-near);
    color: var(--c-near);
  }
  .bar {
    width: 100%;
    height: 8px;
    display: block;
  }
  .consent {
    position: sticky;
    bottom: 0;
    background: var(--c-ground);
    border-radius: 12px;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .actions {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
</style>
