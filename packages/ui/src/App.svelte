<script lang="ts">
  import type { EngineHost, ImportOutcome, ImportStage, MixParams, PlayerCommand, SeparationQuality, TransportSnapshot } from "@workstation/engine-api";
  import Grain from "./Grain.svelte";
  import HomeScreen from "./HomeScreen.svelte";
  import ModelsPanel from "./ModelsPanel.svelte";
  import SessionScreen from "./SessionScreen.svelte";
  import Toasts, { type Toast } from "./Toasts.svelte";
  import { provideServices, type ToastTone } from "./context";
  import { Burst } from "./grain";
  import { enter, installMotionTokens } from "./motion";
  import { clock } from "./clock";
  import { Session } from "./session.svelte";
  import { errorMessage, type AppState, type ViewId } from "./state";
  import { withMix } from "./takes";
  import { installTheme } from "./theme";

  let { host }: { host: EngineHost } = $props();

  installTheme();

  let app = $state<AppState>({ kind: "home", job: { kind: "idle" }, resume: { kind: "none" } });
  let snapshot = $state.raw<TransportSnapshot>({ transport: { kind: "Stopped" }, sample_rate: 44100 });
  let toast = $state<Toast | null>(null);
  /** The model manager, shown over either screen. */
  let modelsPanel = $state<{ kind: "closed" } | { kind: "open" }>({ kind: "closed" });
  const openModels = () => (modelsPanel = { kind: "open" });
  let toastIds = 0;

  const burst = new Burst();
  function showToast(text: string, tone: ToastTone = "info") {
    toast = { id: ++toastIds, text, tone };
  }
  provideServices({ burst, toast: showToast });

  /** The loaded song, whether on screen or waiting behind Home. */
  const loaded = $derived(app.kind === "session" ? app.session : app.resume.kind === "available" ? app.resume.session : null);

  function enterSession(session: Session) {
    app = { kind: "session", session };
  }

  async function runImport(start: (onStage: (s: ImportStage) => void) => Promise<ImportOutcome>) {
    if (app.kind !== "home") return;
    const resume = app.resume;
    app = { kind: "home", job: { kind: "importing", progress: { kind: "Starting" } }, resume };
    try {
      const outcome = await start((stage) => {
        if (app.kind === "home" && app.job.kind === "importing") app = { ...app, job: { kind: "importing", progress: stage } };
      });
      switch (outcome.kind) {
        case "imported":
          enterSession(new Session(outcome.project));
          showToast("Stems ready");
          requestAnimationFrame(() => burst.fire(window.innerWidth / 2, 96));
          break;
        case "cancelled":
          app = { kind: "home", job: { kind: "idle" }, resume };
          break;
      }
    } catch (e) {
      // A failed import may have replaced the engine's song; nothing is left to resume.
      app = { kind: "home", job: { kind: "failed", message: errorMessage(e) }, resume: { kind: "none" } };
    }
  }

  function importUrl(url: string, quality: SeparationQuality) {
    const urlImport = host.urlImport;
    if (urlImport.kind === "available") void runImport((s) => urlImport.importUrl(url, quality, s));
  }

  async function openProject(id: string) {
    if (app.kind !== "home" || host.projects.kind !== "available") return;
    const store = host.projects;
    app = { kind: "home", job: { kind: "opening" }, resume: { kind: "none" } };
    try {
      enterSession(new Session(await store.open(id)));
    } catch (e) {
      app = { kind: "home", job: { kind: "failed", message: errorMessage(e) }, resume: { kind: "none" } };
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

  function back() {
    if (app.kind !== "session") return;
    if (snapshot.transport.kind === "Playing") void send({ kind: "Pause" });
    app = { kind: "home", job: { kind: "idle" }, resume: { kind: "available", session: app.session } };
  }

  async function close() {
    await host.close();
    app = { kind: "home", job: { kind: "idle" }, resume: { kind: "none" } };
  }

  async function save() {
    if (app.kind !== "session" || host.projects.kind !== "available") return;
    const session = app.session;
    session.save = { kind: "saving" };
    try {
      await host.projects.save();
      session.save = { kind: "clean" };
    } catch (e) {
      session.save = { kind: "dirty" };
      showToast(`Couldn't save: ${errorMessage(e)}`, "error");
    }
  }

  /** Mix changes waiting for the next frame: a fader drag sends at most one per track per frame. */
  const pendingMix = new Map<number, MixParams>();
  let mixFrame = 0;

  function setMix(track: number, params: MixParams) {
    if (app.kind !== "session") return;
    const session = app.session;
    // Song tracks first, then takes (the player's order).
    if (track >= session.project.tracks.length + session.project.takes.length) return;
    session.adopt(withMix(session.project, track, params));
    pendingMix.set(track, { mode: params.mode, gain: params.gain, pan: params.pan });
    if (mixFrame === 0)
      mixFrame = requestAnimationFrame(() => {
        mixFrame = 0;
        for (const [t, p] of pendingMix) void send({ kind: "SetMix", track: t, params: p });
        pendingMix.clear();
      });
  }

  function setView(view: ViewId) {
    if (app.kind === "session") app.session.view = view;
  }

  $effect(() => installMotionTokens());

  // The engine pushes the transport (no per-frame polling, AGENTS.md §10.1); the clock
  // extrapolates the playhead between pushes for anything drawn per frame.
  $effect(() => {
    if (!loaded) return;
    return host.subscribeTransport((s) => {
      clock.update(s);
      snapshot = s;
    });
  });
</script>

<!-- Screens crossfade with old and new overlapping in one grid cell (rule 16). -->
<div class="app">
  {#key app.kind}
    <div class="layer" transition:enter>
      {#if app.kind === "home"}
        <HomeScreen
          {host}
          job={app.job}
          resume={app.resume}
          onImportFile={(quality) => runImport((s) => host.importFile(quality, s))}
          onImportUrl={importUrl}
          onOpen={openProject}
          onImportArchive={() => host.projects.kind === "available" && host.projects.archive.kind === "available" && runImport(host.projects.archive.importArchive)}
          onResume={() => app.kind === "home" && app.resume.kind === "available" && enterSession(app.resume.session)}
          onDismiss={() => app.kind === "home" && (app = { ...app, job: { kind: "idle" } })}
          onModels={openModels}
        />
      {:else}
        <SessionScreen
          {host}
          session={app.session}
          {snapshot}
          onBack={back}
          onClose={close}
          onSave={save}
          onCommand={send}
          onMix={setMix}
          onView={setView}
          onModels={openModels}
        />
      {/if}
    </div>
  {/key}
</div>

<div class="overlay" aria-hidden="true"><Grain behaviour={burst} style="width: 100%; height: 100%" /></div>
<Toasts {toast} />
{#if modelsPanel.kind === "open"}
  <ModelsPanel {host} onClose={() => (modelsPanel = { kind: "closed" })} />
{/if}

<style>
  :global(html),
  :global(body) {
    margin: 0;
    height: 100%;
    background: var(--c-ground);
    color: var(--c-text);
    color-scheme: dark;
    font-family: "Geist Variable", Geist, "Helvetica Neue", Arial, sans-serif;
    font-size: 15px;
    line-height: 22px;
    -webkit-font-smoothing: antialiased;
    overflow: hidden;
  }
  /* Scrollbars, where a region does scroll, stay in the ground's palette. */
  :global(*) {
    scrollbar-width: thin;
    scrollbar-color: var(--c-raised) transparent;
  }
  /* A playhead over a canvas lane, moved by `use:playhead` (transform only). */
  :global(.playhead) {
    position: absolute;
    left: 0;
    top: 0;
    width: 2px;
    background: var(--c-text);
    pointer-events: none;
    will-change: transform;
  }
  :global(button) {
    font: inherit;
    color: inherit;
    border: 0;
    margin: 0;
    cursor: pointer;
    background: none;
  }
  /* A disabled button keeps its shape (quiet stays quiet, raised stays raised) and dims its
     label to --c-disabled, which holds 4:1 or better against ground, surface and raised. Only a
     filled white button changes ground, or its dimmed label would sit on white. */
  :global(button:disabled) {
    cursor: default;
    color: var(--c-disabled) !important;
  }
  :global(.primary:disabled) {
    background: var(--c-raised) !important;
  }
  :global(button:focus-visible),
  :global(input:focus-visible) {
    outline: 2px solid var(--c-text);
    outline-offset: 2px;
  }
  /* Rule 4: press feedback. Durations and curves come from motion.ts tokens. */
  :global(.press) {
    transition:
      transform var(--dur-press) var(--ease-out),
      background-color var(--dur-colour) var(--ease-out),
      color var(--dur-colour) var(--ease-out);
  }
  :global(.press:active:not(:disabled)) {
    transform: scale(var(--press-scale));
  }
  :global(.primary) {
    background: var(--c-text);
    color: var(--c-ground);
    font-weight: 500;
  }
  :global(.raised) {
    background: var(--c-raised);
  }
  :global(.quiet) {
    background: transparent;
  }
  :global(.small-button) {
    height: 44px;
    padding: 0 16px;
    border-radius: 12px;
  }
  :global(.quiet:hover:not(:disabled)) {
    background: var(--c-surface);
  }
  :global(.big) {
    height: 44px;
    padding: 0 20px;
    border-radius: 12px;
  }
  :global(.primary.big),
  :global(.raised.big) {
    font-size: 15px;
  }
  :global(.icon) {
    width: 44px;
    height: 44px;
    border-radius: 12px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    flex: none;
  }
  :global(.field) {
    height: 44px;
    box-sizing: border-box;
    padding: 0 16px;
    border: 0;
    border-radius: 12px;
    background: var(--c-surface);
    color: var(--c-text);
    font: inherit;
    transition: background-color var(--dur-colour) var(--ease-out);
  }
  :global(.field::placeholder) {
    color: var(--c-muted);
  }
  :global(.field:focus) {
    background: var(--c-raised);
  }
  :global(.small) {
    font-size: 13px;
    line-height: 18px;
  }
  :global(.muted) {
    color: var(--c-muted);
  }
  :global(.mono) {
    font-family: "Geist Mono Variable", "Geist Mono", ui-monospace, monospace;
    font-size: 13px;
    line-height: 18px;
  }
  :global(.pixel) {
    font-family: "Geist Pixel Circle", "Geist Variable", sans-serif;
  }
  :global(.sr) {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  /* Label swaps rise in with a short blur (design: Components → Play and confirm). */
  :global(.swap) {
    display: inline-block;
    animation: swap-in var(--dur-detail) var(--ease-out);
  }
  @keyframes -global-swap-in {
    from {
      opacity: 0;
      filter: blur(3px);
      transform: translateY(3px);
    }
    to {
      opacity: 1;
      filter: blur(0);
      transform: none;
    }
  }
  .app {
    display: grid;
    height: 100vh;
  }
  .layer {
    grid-area: 1 / 1;
    min-width: 0;
    min-height: 0;
  }
  .overlay {
    position: fixed;
    inset: 0;
    pointer-events: none;
    z-index: 60;
  }
</style>
