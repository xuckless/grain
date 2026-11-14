<script lang="ts">
  import type { EngineHost, LineEdit, LyricsQuery, LyricsUpdate } from "@workstation/engine-api";
  import { services } from "./context";
  import { lookUp, lyricsNext, lyricsStateOf, LYRICS_NUDGE_MS, parseLineTime, queryOf, type LyricsEvent } from "./lyrics";
  import { errorMessage, formatTime, type Session } from "./state";

  /**
   * Where the karaoke lyrics come from (AGENTS.md §6.1): LRCLIB, looked up by artist and title
   * (guessed, editable) the first time the song opens in Karaoke; the global offset nudge; and
   * per-line text and time edits. Results are stored on the project (saved with it), so a song
   * is looked up once. `locked`: a performance is running, so nothing changes under it.
   */
  let { host, session, locked }: { host: EngineHost; session: Session; locked: boolean } = $props();

  const { toast } = services();

  const lyrics = $derived(session.project.lyrics);
  // svelte-ignore state_referenced_locally
  let found = $state(lyricsStateOf(session.project.lyrics));
  // svelte-ignore state_referenced_locally
  let query = $state<LyricsQuery>(queryOf(session.project.lyrics, session.project.song));

  type Editing = { kind: "closed" } | { kind: "open" };
  let editing = $state<Editing>({ kind: "closed" });

  function event(e: LyricsEvent): boolean {
    const next = lyricsNext(found, e);
    if (!next) return false;
    found = next;
    return true;
  }

  async function store(update: LyricsUpdate) {
    const stored = await host.setLyrics(update);
    session.project.lyrics = stored;
    if (session.save.kind === "clean") session.save = { kind: "dirty" };
    event({ kind: "stored", lyrics: stored });
  }

  async function search() {
    const q = { artist: query.artist.trim(), title: query.title.trim() };
    if (q.title === "" || !event({ kind: "fetch", query: q })) return;
    try {
      await store(await lookUp(q, session.project.frames / session.project.sample_rate, (url, init) => fetch(url, init)));
    } catch (e) {
      event({ kind: "fail", message: errorMessage(e) });
    }
  }

  // Look the song up the first time it opens here.
  $effect(() => {
    if (found.kind === "idle") void search();
  });

  /** Sends the current lines with `change` applied (times as shown, offset included). */
  async function edit(offsetMs: number, change: (lines: LineEdit[]) => void) {
    if (lyrics.kind !== "Synced" || locked) return;
    const lines = lyrics.lines.map((l) => ({ lrc_start: l.lrc_start, text: l.text }));
    change(lines);
    try {
      await store({ kind: "Edited", offset_ms: offsetMs, lines });
    } catch (e) {
      toast(`Couldn't change the lyrics: ${errorMessage(e)}`, "error");
    }
  }

  const nudge = (ms: number) => lyrics.kind === "Synced" && void edit(lyrics.offset_ms + ms, () => {});

  function setText(i: number, text: string) {
    if (lyrics.kind === "Synced") void edit(lyrics.offset_ms, (lines) => (lines[i].text = text));
  }

  function setTime(i: number, input: HTMLInputElement) {
    if (lyrics.kind !== "Synced") return;
    const seconds = parseLineTime(input.value);
    if (seconds === null) {
      input.value = formatTime(lyrics.lines[i].start, 2);
      return;
    }
    const offset = lyrics.offset_ms;
    void edit(offset, (lines) => (lines[i].lrc_start = Math.max(0, seconds - offset / 1000)));
  }

  function describe(): string {
    switch (found.kind) {
      case "idle":
        return "";
      case "fetching":
        return "Looking for lyrics on LRCLIB…";
      case "synced":
        return "Synced lyrics from LRCLIB.";
      case "plainOnly":
        return "LRCLIB only has unsynced lyrics for this song, so they're shown without timing.";
      case "notFound":
        return "No lyrics found on LRCLIB. Check the artist and title, then search again.";
      case "failed":
        return `Couldn't reach LRCLIB: ${found.message}`;
    }
  }
</script>

<div class="panel">
  <form
    class="row"
    onsubmit={(e) => {
      e.preventDefault();
      void search();
    }}
  >
    <input class="field" aria-label="Artist" placeholder="Artist" bind:value={query.artist} disabled={locked} />
    <input class="field" aria-label="Title" placeholder="Title" bind:value={query.title} disabled={locked} />
    <button class="press raised small-button" type="submit" disabled={locked || found.kind === "fetching" || query.title.trim() === ""}>
      {found.kind === "failed" ? "Retry" : "Search"}
    </button>
  </form>
  <p class="muted small">{describe()} Only the artist, title and song length are sent.</p>

  {#if lyrics.kind === "Synced"}
    <div class="row">
      <span class="label">Lyrics offset</span>
      <button class="press icon quiet" aria-label="Lyrics {LYRICS_NUDGE_MS} ms earlier" disabled={locked} onclick={() => nudge(-LYRICS_NUDGE_MS)}>−</button>
      <span class="value-sm">{lyrics.offset_ms > 0 ? "+" : ""}{lyrics.offset_ms.toFixed(0)} ms</span>
      <button class="press icon quiet" aria-label="Lyrics {LYRICS_NUDGE_MS} ms later" disabled={locked} onclick={() => nudge(LYRICS_NUDGE_MS)}>+</button>
      <button class="press quiet small-button" disabled={locked} onclick={() => (editing = { kind: editing.kind === "open" ? "closed" : "open" })}>
        {editing.kind === "open" ? "Done editing" : "Edit lines"}
      </button>
    </div>
    {#if editing.kind === "open"}
      <ol class="lines">
        {#each lyrics.lines as line, i (i)}
          <li>
            <input
              class="field mono time"
              aria-label="Line {i + 1} start"
              value={formatTime(line.start, 2)}
              disabled={locked}
              onchange={(e) => setTime(i, e.currentTarget)}
            />
            <input class="field text" aria-label="Line {i + 1} text" value={line.text} disabled={locked} onchange={(e) => setText(i, e.currentTarget.value)} />
          </li>
        {/each}
      </ol>
    {/if}
  {/if}
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .row .field {
    width: 220px;
  }
  .label {
    font-size: 13px;
    line-height: 18px;
    font-weight: 500;
    color: var(--c-muted);
  }
  .value-sm {
    min-width: 72px;
    text-align: center;
    font-variant-numeric: tabular-nums;
    font-weight: 500;
  }
  p {
    margin: 0;
  }
  .lines {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-height: 320px;
    overflow-y: auto;
  }
  .lines li {
    display: flex;
    gap: 8px;
  }
  .time {
    width: 112px;
    height: 36px;
    flex: none;
  }
  .text {
    flex: 1;
    min-width: 0;
    height: 36px;
  }
</style>
