<script lang="ts">
  import type { EngineHost, ProjectSummary, SeparationQuality } from "@workstation/engine-api";
  import Toggle from "./Toggle.svelte";
  import ExtractorStatus from "./ExtractorStatus.svelte";
  import Grain from "./Grain.svelte";
  import TopBar from "./TopBar.svelte";
  import { DotProgress, Gather, Orb, Terrain } from "./grain";
  import { detailDelay, enter, receiveShared, sendShared } from "./motion";
  import { describeProgress, errorMessage, formatDuration, progressFraction, songTitle, type HomeJob, type Resume } from "./state";

  let {
    host,
    job,
    resume,
    onImportFile,
    onImportUrl,
    onOpen,
    onImportArchive,
    onResume,
    onDismiss,
  }: {
    host: EngineHost;
    job: HomeJob;
    resume: Resume;
    onImportFile: (quality: SeparationQuality) => void;
    onImportUrl: (url: string, quality: SeparationQuality) => void;
    onOpen: (id: string) => void;
    onImportArchive: () => void;
    onResume: () => void;
    onDismiss: () => void;
  } = $props();

  type Recent =
    | { kind: "loading" }
    | { kind: "loaded"; items: ProjectSummary[] }
    | { kind: "failed"; message: string }
    | { kind: "unavailable"; reason: string };
  let recent = $state<Recent>({ kind: "loading" });
  let url = $state("");
  let quality = $state<SeparationQuality>("Fast");

  const logo = new Orb(2.4);
  const hero = new Orb(7);
  const terrain = new Terrain();
  const gather = new Gather();
  const bar = new DotProgress();

  const busy = $derived(job.kind === "importing" || job.kind === "opening");
  const progress = $derived(job.kind === "importing" ? progressFraction(job.progress) : job.kind === "opening" ? 1 : 0);
  $effect(() => {
    gather.progress = progress;
    bar.progress = progress;
  });

  $effect(() => {
    const store = host.projects;
    if (store.kind === "unavailable") {
      recent = { kind: "unavailable", reason: store.reason };
      return;
    }
    store.list().then(
      (items) => (recent = { kind: "loaded", items }),
      (e) => (recent = { kind: "failed", message: errorMessage(e) }),
    );
  });

  const day = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });
  const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
  function saved(ms: number): string {
    const d = new Date(ms);
    return d.toDateString() === new Date().toDateString() ? `Today ${clock.format(d)}` : day.format(d);
  }

  function ripple(e: PointerEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    hero.ripple(e.clientX - r.left, e.clientY - r.top);
  }
</script>

<div class="screen">
  <div class="terrain" aria-hidden="true"><Grain behaviour={terrain} style="width: 100%; height: 220px" /></div>

  <TopBar chrome={host.chrome}>
    {#snippet left()}
      <Grain behaviour={logo} style="width: 28px; height: 28px; flex: none" label="Grain" />
      <span class="wordmark pixel">grain</span>
    {/snippet}
    {#snippet right()}
      {#if resume.kind === "available"}
        {@const title = songTitle(resume.session.project)}
        <button class="press raised continue" onclick={onResume}>
          <span class="muted">Continue</span>
          <span class="name" in:receiveShared|global={{ key: `title:${title}` }} out:sendShared|global={{ key: `title:${title}` }}>{title}</span>
        </button>
      {/if}
    {/snippet}
  </TopBar>

  <main>
    <section class="hero">
      <div class="copy">
        <h1 class="pixel">Hear every voice</h1>
        <p class="muted lead">Split a song into its stems, then sing it back and see where you were sharp, flat, early or late.</p>

        <!-- Job states crossfade with overlap (rule 16). -->
        <div class="stage">
          {#key job.kind}
            <div class="layer" transition:enter>
              {#if job.kind === "idle"}
                <div class="actions">
                  <button class="press primary big" onclick={() => onImportFile(quality)}>Open song…</button>
                  {#if host.projects.kind === "available" && host.projects.archive.kind === "available"}
                    <button class="press raised big" onclick={onImportArchive}>Open project file…</button>
                  {/if}
                  {#if host.urlImport.kind === "available"}
                    <form
                      class="link"
                      onsubmit={(e) => {
                        e.preventDefault();
                        if (url.trim() !== "") onImportUrl(url.trim(), quality);
                      }}
                    >
                      <label class="sr" for="song-link">Song link</label>
                      <input id="song-link" class="field" type="url" placeholder="Paste a YouTube link" bind:value={url} />
                      <button class="press raised big" type="submit" disabled={url.trim() === ""}>Import</button>
                    </form>
                  {:else if host.urlImport.kind === "missing"}
                    <span class="muted small">{host.urlImport.reason}</span>
                  {/if}
                </div>
                <div class="quality">
                  {#if host.separation.kind === "choice"}
                    <Toggle
                      checked={quality === "BestVocals"}
                      label="Best vocals: slower, a cleaner vocal stem for songs with a backing track"
                      onToggle={(on) => (quality = on ? "BestVocals" : "Fast")}
                    />
                  {:else}
                    <span class="muted small">{host.separation.reason}</span>
                  {/if}
                </div>
                {#if host.urlImport.kind === "available"}
                  <ExtractorStatus extractor={host.urlImport.extractor} />
                {/if}
              {:else if job.kind === "failed"}
                <div class="status">
                  <strong>That didn't work</strong>
                  <span class="muted">{job.message}</span>
                  <button class="press raised big" onclick={onDismiss}>Try again</button>
                </div>
              {:else}
                <div class="status">
                  <Grain behaviour={bar} style="width: 420px; max-width: 100%; height: 56px" />
                  <div class="progress-label">
                    <span>{job.kind === "importing" ? describeProgress(job.progress) : "Opening project"}</span>
                    <span class="mono muted">{Math.round(progress * 100)}%</span>
                  </div>
                </div>
              {/if}
            </div>
          {/key}
        </div>
      </div>

      <div class="art">
        {#key busy}
          <div class="layer" transition:enter>
            {#if busy}
              <Grain behaviour={gather} style="width: 100%; height: 100%" label="Separating" />
            {:else}
              <button class="orb" aria-label="Grain mark: press to send a ripple through it" onpointerdown={ripple}>
                <Grain behaviour={hero} style="width: 100%; height: 100%" />
              </button>
            {/if}
          </div>
        {/key}
      </div>
    </section>

    <section class="recent">
      <h2>Recent</h2>
      {#if recent.kind === "loaded" && recent.items.length > 0}
        <div class="rows">
          {#each recent.items as item, i (item.id)}
            {@const title = songTitle(item)}
            <button class="press row" disabled={busy} onclick={() => onOpen(item.id)} in:enter={{ delay: detailDelay(i) }}>
              <span class="title" in:receiveShared|global={{ key: `title:${title}` }} out:sendShared|global={{ key: `title:${title}` }}>{title}</span>
              <span class="mono muted meta">
                {formatDuration(item.duration_seconds)} · {item.bpm.toFixed(0)} BPM{item.key ? ` · ${item.key}` : ""} · {item.tracks} {item.tracks === 1 ? "track" : "stems"}
              </span>
              <span class="muted when">{saved(item.saved_ms)}</span>
            </button>
          {/each}
        </div>
      {:else if recent.kind === "loaded"}
        <p class="muted">Songs you open will appear here.</p>
      {:else if recent.kind === "unavailable"}
        <p class="muted">{recent.reason}</p>
      {:else if recent.kind === "failed"}
        <p class="muted">Couldn't read your saved songs: {recent.message}</p>
      {/if}
    </section>
  </main>
</div>

<style>
  .screen {
    position: relative;
    height: 100vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .terrain {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    pointer-events: none;
  }
  .wordmark {
    font-size: 26px;
    line-height: 28px;
    margin-left: 6px;
  }
  .continue {
    height: 44px;
    padding: 0 16px;
    border-radius: 12px;
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: 360px;
  }
  .continue .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  main {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: auto;
    box-sizing: border-box;
    padding: 24px 64px 120px;
    display: flex;
    flex-direction: column;
    gap: 56px;
  }
  .hero {
    display: flex;
    align-items: center;
    gap: 48px;
  }
  .copy {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }
  h1 {
    margin: 0;
    font-size: 104px;
    line-height: 112px;
    font-weight: 400;
    max-width: 760px;
  }
  .lead {
    margin: 0;
    max-width: 520px;
  }
  .stage {
    display: grid;
    min-height: 120px;
    margin-top: 12px;
  }
  .layer {
    grid-area: 1 / 1;
    min-width: 0;
  }
  .actions {
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
    align-items: center;
  }
  .quality {
    margin-top: 16px;
  }
  .link {
    display: flex;
    gap: 8px;
    flex: 1;
    min-width: 320px;
    max-width: 520px;
  }
  .link .field {
    flex: 1;
  }
  .status {
    display: flex;
    flex-direction: column;
    gap: 12px;
    align-items: flex-start;
  }
  .status strong {
    font-size: 20px;
    line-height: 28px;
    font-weight: 500;
  }
  .progress-label {
    width: 420px;
    max-width: 100%;
    display: flex;
    justify-content: space-between;
    gap: 16px;
  }
  .art {
    width: 380px;
    height: 380px;
    flex: none;
    display: grid;
  }
  .art .layer {
    height: 100%;
  }
  .orb {
    display: block;
    width: 100%;
    height: 100%;
    padding: 0;
    background: none;
    cursor: pointer;
    border-radius: 50%;
  }
  .recent {
    display: flex;
    flex-direction: column;
    gap: 20px;
    padding-top: 20px;
    border-top: 0.5px solid var(--c-text);
    max-width: 960px;
  }
  h2 {
    margin: 0;
    font-size: 17px;
    line-height: 24px;
    font-weight: 500;
  }
  .rows {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .row {
    height: 68px;
    box-sizing: border-box;
    padding: 0 24px;
    background: var(--c-surface);
    border-radius: 12px;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto 120px;
    align-items: center;
    gap: 24px;
    text-align: left;
    transition:
      background-color var(--dur-colour) var(--ease-out),
      transform var(--dur-press) var(--ease-out);
  }
  .row:hover:not(:disabled) {
    background: var(--c-raised);
  }
  .title {
    font-size: 15px;
    line-height: 22px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .when {
    text-align: right;
    font-size: 13px;
  }
  p {
    margin: 0;
  }
  @media (max-width: 1100px) {
    h1 {
      font-size: 72px;
      line-height: 80px;
    }
    .art {
      width: 280px;
      height: 280px;
    }
  }
</style>
