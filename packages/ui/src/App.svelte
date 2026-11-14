<script lang="ts">
  import type { EngineHost, ImportOutcome, MixParams, PlayerCommand, ScoreSettings, TransportSnapshot } from "@workstation/engine-api";
  import Timeline from "./Timeline.svelte";
  import TransportBar from "./TransportBar.svelte";
  import ScorePanel from "./ScorePanel.svelte";
  import { describeProgress, errorMessage, type AppState } from "./state";
  import { enter, installMotionTokens } from "./motion";

  let { host }: { host: EngineHost } = $props();

  let app: AppState = $state({ kind: "empty" });
  let snapshot: TransportSnapshot = $state({ transport: { kind: "Stopped" }, sample_rate: 44100 });
  let url = $state("");

  async function runImport(start: (onStage: (s: import("@workstation/engine-api").ImportStage) => void) => Promise<ImportOutcome>) {
    const previous = app;
    app = { kind: "importing", progress: { kind: "Starting" } };
    try {
      const outcome = await start((stage) => {
        if (app.kind === "importing") app = { kind: "importing", progress: stage };
      });
      switch (outcome.kind) {
        case "imported":
          app = { kind: "ready", project: outcome.project, score: { kind: "none" } };
          break;
        case "cancelled":
          app = previous;
          break;
      }
    } catch (e) {
      app = { kind: "failed", message: errorMessage(e) };
    }
  }

  async function send(command: PlayerCommand) {
    try {
      await host.send(command);
    } catch (e) {
      // Illegal transitions are rejected by the engine; the transport poll shows the real state.
      console.warn(errorMessage(e));
    }
  }

  function setMix(track: number, params: MixParams) {
    if (app.kind !== "ready") return;
    app.project.tracks[track].mix = params;
    void send({ kind: "SetMix", track, params });
  }

  async function score(settings: ScoreSettings) {
    if (app.kind !== "ready") return;
    const ready = app;
    ready.score = { kind: "scoring" };
    try {
      const outcome = await host.scoreTake(settings);
      ready.score = outcome.kind === "scored" ? { kind: "scored", report: outcome.report } : { kind: "none" };
    } catch (e) {
      ready.score = { kind: "failed", message: errorMessage(e) };
    }
  }

  $effect(() => installMotionTokens());

  // Poll the engine's transport once per animation frame while a song is loaded.
  $effect(() => {
    if (app.kind !== "ready") return;
    let alive = true;
    const tick = async () => {
      if (!alive) return;
      try {
        snapshot = await host.transport();
      } catch (e) {
        console.warn(errorMessage(e));
      }
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
    return () => {
      alive = false;
    };
  });
</script>

<main>
  <header>
    <h1>Workstation</h1>
    <button onclick={() => runImport((s) => host.importFile(s))} disabled={app.kind === "importing"}>Open song…</button>
    {#if host.urlImport.kind === "available"}
      {@const urlImport = host.urlImport}
      <form
        onsubmit={(e) => {
          e.preventDefault();
          void runImport((s) => urlImport.importUrl(url, s));
        }}
      >
        <input placeholder="YouTube link" bind:value={url} />
        <button type="submit" disabled={app.kind === "importing" || url.trim() === ""}>Import</button>
      </form>
    {/if}
  </header>

  <!-- State swaps crossfade with old and new layers overlapping in one grid cell (motion rule 16). -->
  <div class="stage">
  {#key app.kind}
  <div class="layer" transition:enter>
  {#if app.kind === "empty"}
    <p class="hint">Open an audio file{host.urlImport.kind === "available" ? " or paste a YouTube link" : ""} to split it into stems.</p>
  {:else if app.kind === "importing"}
    <p class="hint">{describeProgress(app.progress)}</p>
  {:else if app.kind === "failed"}
    <p class="error">{app.message}</p>
  {:else}
    <TransportBar project={app.project} {snapshot} onCommand={send} />
    {#if app.project.analysis.reference_source === "FullMix"}
      <p class="warning">No separation model installed: the reference melody was tracked on the full mix and will be unreliable.</p>
    {/if}
    <Timeline project={app.project} {snapshot} onSeek={(frame) => send({ kind: "Seek", frame })} onMix={setMix} />
    <ScorePanel project={app.project} score={app.score} onScore={score} onSeek={(frame) => send({ kind: "Seek", frame })} sampleRate={snapshot.sample_rate} />
  {/if}
  </div>
  {/key}
  </div>
</main>

<style>
  :global(:root) {
    --bg: #f6f6f4;
    --panel: #ffffff;
    --text: #1c1c1e;
    --muted: #6b6b70;
    --line: #dcdcd8;
    --accent: #3b6fd8;
    --good: #2f9e57;
    --bad: #d24b3e;
    --warn: #c98a1b;
    --wave: #7d8aa6;
    color-scheme: light dark;
  }
  @media (prefers-color-scheme: dark) {
    :global(:root) {
      --bg: #151517;
      --panel: #1f1f22;
      --text: #ececee;
      --muted: #9a9aa2;
      --line: #34343a;
      --accent: #6d9bff;
      --good: #4cc47a;
      --bad: #f06b5e;
      --warn: #e5ad45;
      --wave: #8e9bbb;
    }
  }
  :global(body) {
    margin: 0;
    background: var(--bg);
    color: var(--text);
    font: 14px/1.4 system-ui, sans-serif;
  }
  :global(button),
  :global(input),
  :global(select) {
    font: inherit;
    color: inherit;
    background: var(--panel);
    border: 1px solid var(--line);
    border-radius: 6px;
    padding: 4px 10px;
  }
  :global(button:disabled) {
    opacity: 0.5;
  }
  /* Motion rule 4: press feedback. Timing comes from motion.ts tokens. */
  :global(button) {
    transition: transform var(--motion-press-dur) var(--motion-ease-enter);
  }
  :global(button:active:not(:disabled)) {
    transform: scale(var(--motion-press-scale));
  }
  /* Motion rule 5: range thumbs stretch along their travel (see knobStretch in motion.ts). */
  :global(input[type="range"]) {
    appearance: none;
    height: 4px;
    border-radius: 2px;
    background: var(--line);
    border: none;
    padding: 0;
  }
  :global(input[type="range"]::-webkit-slider-thumb) {
    appearance: none;
    width: 12px;
    height: 12px;
    border-radius: 6px;
    background: var(--accent);
    transform: scaleX(var(--knob-stretch, 1));
    transition: transform var(--knob-settle, 0ms);
  }
  :global(input[type="range"]::-moz-range-thumb) {
    width: 12px;
    height: 12px;
    border: none;
    border-radius: 6px;
    background: var(--accent);
    transform: scaleX(var(--knob-stretch, 1));
    transition: transform var(--knob-settle, 0ms);
  }
  main {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .stage {
    display: grid;
  }
  .layer {
    grid-area: 1 / 1;
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  header {
    display: flex;
    gap: 12px;
    align-items: center;
    flex-wrap: wrap;
  }
  header form {
    display: flex;
    gap: 6px;
  }
  header input {
    width: min(360px, 60vw);
  }
  h1 {
    font-size: 18px;
    margin: 0 8px 0 0;
  }
  .hint {
    color: var(--muted);
  }
  .error {
    color: var(--bad);
  }
  .warning {
    color: var(--warn);
    margin: 0;
  }
</style>
