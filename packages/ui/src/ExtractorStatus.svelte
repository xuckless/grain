<script lang="ts">
  import type { Extractor, ExtractorOrigin } from "@workstation/engine-api";
  import { errorMessage } from "./state";

  /** Which yt-dlp YouTube import uses, and updating it (AGENTS.md §4.1). */
  let { extractor }: { extractor: Extractor } = $props();

  type Info = { version: string; origin: ExtractorOrigin };
  type Status =
    | { kind: "checking" }
    | { kind: "ready"; info: Info }
    | { kind: "updating"; info: Info }
    | { kind: "updated"; info: Info }
    | { kind: "upToDate"; info: Info }
    | { kind: "failed"; info: Info | null; reason: string };

  let status = $state<Status>({ kind: "checking" });

  $effect(() => {
    const origin = extractor.origin;
    extractor.version().then(
      (version) => (status = { kind: "ready", info: { version, origin } }),
      (e) => (status = { kind: "failed", info: null, reason: errorMessage(e) }),
    );
  });

  async function update(info: Info) {
    status = { kind: "updating", info };
    try {
      const r = await extractor.update();
      const next = { version: r.version, origin: r.origin };
      status = r.kind === "updated" ? { kind: "updated", info: next } : { kind: "upToDate", info: next };
    } catch (e) {
      status = { kind: "failed", info, reason: errorMessage(e) };
    }
  }

  const info = $derived(status.kind === "checking" ? null : status.info);
  const note = $derived(
    status.kind === "updated"
      ? `Updated to ${status.info.version}`
      : status.kind === "upToDate"
        ? "Already up to date"
        : status.kind === "failed"
          ? status.reason
          : null,
  );
</script>

<div class="extractor small">
  <span class="mono muted">
    {#if info}yt-dlp {info.version} · {info.origin}{:else if status.kind === "checking"}yt-dlp …{/if}
  </span>
  {#if info && info.origin !== "system"}
    <button class="press quiet small-button" disabled={status.kind === "updating"} onclick={() => update(info)}>
      {#key status.kind === "updating"}<span class="swap">{status.kind === "updating" ? "Updating…" : "Update extractor"}</span>{/key}
    </button>
  {:else if info}
    <span class="muted">Update it with your package manager</span>
  {/if}
  {#if note}
    {#key note}<span class="swap muted note" role="status">{note}</span>{/key}
  {/if}
</div>

<style>
  .extractor {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
    margin-top: 12px;
  }
  .note {
    flex-basis: 100%;
  }
</style>
