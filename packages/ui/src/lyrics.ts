// Karaoke lyrics in the UI (AGENTS.md §6.1): guessing what to look a song up by, the LRCLIB
// lookup, the lyrics state machine, and where the singer is in the timed lines. Parsing and
// syllable timing happen in the engine (`engine-core::lyrics`, via `host.setLyrics`). Pure
// functions with no Svelte or host imports, so they are unit-tested under Node.

import type { LyricLine, LyricsQuery, LyricsUpdate, ProjectLyrics, SongSource } from "@workstation/engine-api";

// ---------------------------------------------------------------------------------------------
// What to look a song up by.

/** Video-title clutter that isn't part of a song's name. */
const CLUTTER = /\s*[([【「](official|music video|mv|lyric|lyrics|audio|video|hd|4k|full|ver\.?|version)[^)\]】」]*[)\]】」]\s*/gi;

/** Artist and title from a file name (`Artist - Title.mp3`, `artist-title.mp3`) or a video title.
 * Without a separator the whole name is the title. The user can edit both before searching. */
export function guessQuery(song: SongSource): LyricsQuery {
  const name = (song.kind === "File" ? song.name.replace(/\.[^.]+$/, "") : song.title).replace(CLUTTER, " ").trim();
  const spaced = name.split(/\s+[-–—|]\s+/);
  const parts = spaced.length > 1 ? spaced : name.split(/[-_]/);
  if (parts.length < 2) return { artist: "", title: name };
  const [artist, ...rest] = parts;
  return { artist: artist.trim(), title: rest.join(" ").replace(/_/g, " ").trim() };
}

// ---------------------------------------------------------------------------------------------
// LRCLIB (https://lrclib.net/docs): free, no key. Browsers can't set User-Agent, so the client is
// named in `Lrclib-Client`, which LRCLIB accepts (and allows through CORS). Only the artist,
// title and duration leave the machine.

export const LRCLIB = "https://lrclib.net/api";
const CLIENT = { "Lrclib-Client": "Grain/0.1" };

type LrclibRecord = { trackName?: string; artistName?: string; duration?: number; syncedLyrics?: string | null; plainLyrics?: string | null; instrumental?: boolean };

export type Fetcher = (url: string, init: { headers: Record<string, string> }) => Promise<{ ok: boolean; status: number; json(): Promise<unknown> }>;

/** Looks a song up: `/api/get` (exact match within a couple of seconds of `duration`), then
 * `/api/search`, preferring synced lyrics and the closest duration. Throws on network errors. */
export async function lookUp(query: LyricsQuery, duration: number, fetcher: Fetcher): Promise<LyricsUpdate> {
  const params = (p: Record<string, string>) => new URLSearchParams(Object.entries(p).filter(([, v]) => v !== "")).toString();
  const get = await fetcher(`${LRCLIB}/get?${params({ artist_name: query.artist, track_name: query.title, duration: String(Math.round(duration)) })}`, { headers: CLIENT });
  if (get.ok) {
    const found = toUpdate(query, (await get.json()) as LrclibRecord);
    if (found) return found;
  } else if (get.status !== 404) {
    throw new Error(`LRCLIB answered ${get.status}`);
  }
  const search = await fetcher(`${LRCLIB}/search?${params({ track_name: query.title, artist_name: query.artist })}`, { headers: CLIENT });
  if (!search.ok) throw new Error(`LRCLIB answered ${search.status}`);
  const results = ((await search.json()) as LrclibRecord[]).filter((r) => !r.instrumental && (r.syncedLyrics || r.plainLyrics));
  const distance = (r: LrclibRecord) => Math.abs((r.duration ?? Infinity) - duration);
  results.sort((a, b) => Number(!a.syncedLyrics) - Number(!b.syncedLyrics) || distance(a) - distance(b));
  return (results[0] && toUpdate(query, results[0])) ?? { kind: "NotFound", query };
}

function toUpdate(query: LyricsQuery, r: LrclibRecord): LyricsUpdate | null {
  if (r.instrumental || (!r.syncedLyrics && !r.plainLyrics)) return null;
  return { kind: "Fetched", query, synced: r.syncedLyrics ?? null, plain: r.plainLyrics ?? null };
}

// ---------------------------------------------------------------------------------------------
// Lyrics state machine: Idle → Fetching → Synced | PlainOnly | NotFound | Failed.

export type LyricsState =
  | { kind: "idle" }
  | { kind: "fetching"; query: LyricsQuery }
  | { kind: "synced" }
  | { kind: "plainOnly" }
  | { kind: "notFound" }
  | { kind: "failed"; message: string };

export type LyricsEvent = { kind: "fetch"; query: LyricsQuery } | { kind: "stored"; lyrics: ProjectLyrics } | { kind: "fail"; message: string };

/** The state a song's stored lyrics put the view in. */
export function lyricsStateOf(lyrics: ProjectLyrics): LyricsState {
  switch (lyrics.kind) {
    case "NotFetched":
      return { kind: "idle" };
    case "NotFound":
      return { kind: "notFound" };
    case "Plain":
      return { kind: "plainOnly" };
    case "Synced":
      return { kind: "synced" };
  }
}

export function lyricsNext(s: LyricsState, e: LyricsEvent): LyricsState | null {
  switch (e.kind) {
    case "fetch":
      return s.kind === "fetching" ? null : { kind: "fetching", query: e.query };
    case "stored":
      // Edits arrive while synced; lookups while fetching.
      return s.kind === "fetching" || s.kind === "synced" ? lyricsStateOf(e.lyrics) : null;
    case "fail":
      return s.kind === "fetching" ? { kind: "failed", message: e.message } : null;
  }
}

/** The query shown in the search fields: what the lyrics were found by, else a guess. */
export function queryOf(lyrics: ProjectLyrics, song: SongSource): LyricsQuery {
  return lyrics.kind === "NotFetched" ? guessQuery(song) : { ...lyrics.query };
}

// ---------------------------------------------------------------------------------------------
// Where the singer is.

/** Index of the line being sung at `seconds` (the last line started), or -1 before the first. */
export function currentLine(lines: readonly LyricLine[], seconds: number): number {
  let lo = 0, hi = lines.length - 1, found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (lines[mid].start <= seconds) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found;
}

/** How much of a syllable has been sung at `seconds`, 0–1 (drives the karaoke wipe). */
export function sungFraction(start: number, end: number, seconds: number): number {
  if (seconds <= start) return 0;
  if (seconds >= end || end <= start) return 1;
  return (seconds - start) / (end - start);
}

/** Parses a line time as `formatTime(t, 2)` shows it, `mm:ss.xx` (or plain seconds); `null`
 * when it isn't a time. */
export function parseLineTime(text: string): number | null {
  const m = /^\s*(?:(\d+):)?(\d+(?:\.\d+)?)\s*$/.exec(text);
  if (!m) return null;
  const seconds = Number(m[2]);
  if (m[1] !== undefined && seconds >= 60) return null;
  return Number(m[1] ?? 0) * 60 + seconds;
}

/** Step of the lyrics offset nudge, ms. */
export const LYRICS_NUDGE_MS = 100;
