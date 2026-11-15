<script lang="ts">
  import type { EngineHost, ExportFormat, ExportRange, FormatOffer } from "@workstation/engine-api";
  import Icon from "./Icon.svelte";
  import { exportNext, exportTracks, FORMAT_LABEL, type ExportEvent, type ExportState } from "./exporting";
  import { popover } from "./motion";
  import { errorMessage, songTitle, type Session } from "./state";

  /**
   * Export… (AGENTS.md §8.8): a checklist of tracks (ticked as the mix is heard now, with the ★
   * master take), the format, and the range (the song or the loop), then the mixdown with every
   * take through its processing chain. Its states are explicit: idle, rendering (progress),
   * encoding, done (where it was saved) or failed.
   */
  let { host, session, loop, onClose }: { host: EngineHost; session: Session; loop: { start: number; end: number } | null; onClose: () => void } = $props();

  // svelte-ignore state_referenced_locally
  const rows = exportTracks(session.project);
  let picked = $state<Record<number, boolean>>(Object.fromEntries(rows.map((r) => [r.index, r.checked])));
  let format = $state<ExportFormat>("Wav24");
  let range = $state<"Song" | "Loop">("Song");
  let offers = $state<FormatOffer[]>([]);
  let job = $state<ExportState>({ kind: "idle" });
  let root = $state<HTMLDivElement | undefined>();

  $effect(() => {
    host.exporter.formats().then((f) => (offers = f), (e) => (job = { kind: "failed", message: errorMessage(e) }));
  });

  const event = (e: ExportEvent) => (job = exportNext(job, e) ?? job);
  const running = $derived(job.kind === "rendering" || job.kind === "encoding");
  const chosen = $derived(rows.filter((r) => picked[r.index]).map((r) => r.index));

  async function run() {
    if (running || chosen.length === 0) return;
    event({ kind: "start" });
    const r: ExportRange = range === "Loop" && loop ? { kind: "Loop", start: loop.start, end: loop.end } : { kind: "Song" };
    try {
      const out = await host.exporter.run({ tracks: chosen, format, range: r }, songTitle(session.project), (p) =>
        event(p.kind === "Rendering" ? { kind: "progress", fraction: p.fraction } : { kind: "encoding" }),
      );
      event(out.kind === "saved" ? { kind: "saved", path: out.path } : { kind: "cancelled" });
    } catch (e) {
      event({ kind: "fail", message: errorMessage(e) });
    }
  }

  $effect(() => {
    const away = (e: PointerEvent) => {
      if (!running && root && !root.contains(e.target as Node) && !(e.target as Element).closest?.("[data-export-trigger]")) onClose();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && !running && onClose();
    window.addEventListener("pointerdown", away);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("pointerdown", away);
      window.removeEventListener("keydown", esc);
    };
  });
</script>

<div class="dialog" role="dialog" aria-label="Export" bind:this={root} transition:popover>
  <div class="head">
    <strong>Export mixdown</strong>
    <button class="press icon quiet" aria-label="Close" disabled={running} onclick={onClose}><Icon name="close" /></button>
  </div>

  <fieldset disabled={running}>
    <legend>Tracks</legend>
    {#each rows as r (r.index)}
      <label class="check"><input type="checkbox" bind:checked={picked[r.index]} /> {r.label}</label>
    {/each}
  </fieldset>

  <fieldset disabled={running}>
    <legend>Format</legend>
    {#each offers as o (o.format)}
      <label class="check" title={o.availability.kind === "unavailable" ? o.availability.reason : undefined}>
        <input type="radio" name="format" value={o.format} bind:group={format} disabled={o.availability.kind === "unavailable"} />
        {FORMAT_LABEL[o.format]}
        {#if o.availability.kind === "unavailable"}<span class="muted small">Unavailable: {o.availability.reason}</span>{/if}
      </label>
    {/each}
  </fieldset>

  <fieldset disabled={running}>
    <legend>Range</legend>
    <label class="check"><input type="radio" name="range" value="Song" bind:group={range} /> Whole song</label>
    <label class="check" title={loop ? undefined : "Set a loop in Karaoke first"}>
      <input type="radio" name="range" value="Loop" bind:group={range} disabled={!loop} /> Loop {loop ? `(${loop.start.toFixed(1)}–${loop.end.toFixed(1)} s)` : ""}
    </label>
  </fieldset>

  <div class="foot">
    <span class="muted small status" aria-live="polite">
      {#if job.kind === "rendering"}
        Rendering {Math.round(job.fraction * 100)}%
      {:else if job.kind === "encoding"}
        Encoding…
      {:else if job.kind === "done"}
        Saved {job.path}
      {:else if job.kind === "failed"}
        Couldn't export: {job.message}
      {:else}
        Takes are exported through their pitch edits, auto-tune, clean-up and vocal chain.
      {/if}
    </span>
    <button class="press primary go" disabled={running || chosen.length === 0} onclick={run}>{running ? "Exporting" : "Export"}</button>
  </div>
  {#if job.kind === "rendering"}<div class="bar" style:transform="scaleX({job.fraction})"></div>{/if}
</div>

<style>
  .dialog {
    position: absolute;
    right: 0;
    top: 52px;
    width: 380px;
    box-sizing: border-box;
    padding: 16px 18px;
    background: var(--c-raised);
    border-radius: 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    transform-origin: top right;
    z-index: 40;
    font-size: 14px;
    line-height: 21px;
    overflow: hidden;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  strong {
    font-weight: 500;
    font-size: 15px;
  }
  fieldset {
    border: 0;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  legend {
    font-size: 13px;
    color: var(--c-muted);
    margin-bottom: 4px;
    font-weight: 500;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    min-height: 28px;
  }
  .small {
    font-size: 12px;
    line-height: 16px;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .status {
    flex: 1;
    overflow-wrap: anywhere;
  }
  .go {
    height: 40px;
    padding: 0 18px;
    border-radius: 10px;
  }
  .bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 3px;
    background: var(--c-text);
    transform-origin: left;
  }
</style>
