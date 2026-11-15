// Run with `pnpm --filter @workstation/ui test`. The LRCLIB network is mocked.
import assert from "node:assert/strict";
import { test } from "node:test";
import type { LyricLine, LyricsQuery } from "@workstation/engine-api";
import { currentLine, guessQuery, lookUp, lyricsNext, lyricsStateOf, parseLineTime, sungFraction, type Fetcher, type LyricsState } from "./lyrics.ts";

test("guesses artist and title from file names and video titles", () => {
  assert.deepEqual(guessQuery({ kind: "File", name: "eill-finale.mp3" }), { artist: "eill", title: "finale" });
  assert.deepEqual(guessQuery({ kind: "File", name: "Some Artist - Long Song Name.flac" }), { artist: "Some Artist", title: "Long Song Name" });
  assert.deepEqual(guessQuery({ kind: "File", name: "Solo.wav" }), { artist: "", title: "Solo" });
  assert.deepEqual(guessQuery({ kind: "YouTube", url: "u", title: "Band – Tune (Official Music Video)" }), { artist: "Band", title: "Tune" });
});

type Call = { url: string; headers: Record<string, string> };

function mock(routes: Record<string, { status: number; body?: unknown }>, calls: Call[]): Fetcher {
  return async (url, init) => {
    calls.push({ url, headers: init.headers });
    const path = new URL(url).pathname.split("/").pop()!;
    const r = routes[path] ?? { status: 404 };
    return { ok: r.status >= 200 && r.status < 300, status: r.status, json: async () => r.body };
  };
}

const query: LyricsQuery = { artist: "eill", title: "finale" };

test("uses an exact /get match when there is one", async () => {
  const calls: Call[] = [];
  const update = await lookUp(query, 225.4, mock({ get: { status: 200, body: { syncedLyrics: "[00:01.00]x", plainLyrics: "x" } } }, calls));
  assert.deepEqual(update, { kind: "Fetched", query, synced: "[00:01.00]x", plain: "x" });
  assert.equal(calls.length, 1);
  assert.match(calls[0].url, /\/get\?artist_name=eill&track_name=finale&duration=225$/);
  assert.equal(calls[0].headers["Lrclib-Client"], "Grain/0.1");
});

test("falls back to /search, preferring synced lyrics and the closest duration", async () => {
  const calls: Call[] = [];
  const results = [
    { duration: 226, syncedLyrics: null, plainLyrics: "plain only" },
    { duration: 300, syncedLyrics: "[00:01.00]far", plainLyrics: "far" },
    { duration: 242, syncedLyrics: "[00:01.00]near", plainLyrics: "near" },
    { duration: 225, instrumental: true, syncedLyrics: null, plainLyrics: null },
  ];
  const update = await lookUp(query, 225.4, mock({ search: { status: 200, body: results } }, calls));
  assert.deepEqual(update, { kind: "Fetched", query, synced: "[00:01.00]near", plain: "near" });
  assert.equal(calls.length, 2);
  assert.match(calls[1].url, /\/search\?track_name=finale&artist_name=eill$/);
});

test("reports not found, and throws on server errors", async () => {
  assert.deepEqual(await lookUp(query, 10, mock({ search: { status: 200, body: [] } }, [])), { kind: "NotFound", query });
  await assert.rejects(lookUp(query, 10, mock({ get: { status: 500 } }, [])), /500/);
});

test("lyrics state machine", () => {
  const idle = lyricsStateOf({ kind: "NotFetched" });
  assert.deepEqual(idle, { kind: "idle" });
  const fetching = lyricsNext(idle, { kind: "fetch", query })!;
  assert.equal(fetching.kind, "fetching");
  assert.equal(lyricsNext(fetching, { kind: "fetch", query }), null, "one lookup at a time");
  assert.deepEqual(lyricsNext(fetching, { kind: "stored", lyrics: { kind: "Synced", query, offset_ms: 0, lines: [] } }), { kind: "synced" });
  assert.deepEqual(lyricsNext(fetching, { kind: "stored", lyrics: { kind: "Plain", query, lines: ["a"] } }), { kind: "plainOnly" });
  assert.deepEqual(lyricsNext(fetching, { kind: "stored", lyrics: { kind: "NotFound", query } }), { kind: "notFound" });
  const failed = lyricsNext(fetching, { kind: "fail", message: "offline" })!;
  assert.deepEqual(failed, { kind: "failed", message: "offline" });
  assert.equal(lyricsNext(failed, { kind: "fail", message: "again" }), null);
  assert.equal(lyricsNext({ kind: "notFound" } as LyricsState, { kind: "stored", lyrics: { kind: "NotFound", query } }), null);
  assert.equal(lyricsNext(failed, { kind: "fetch", query })!.kind, "fetching", "retry");
});

test("finds the current line and the wipe position", () => {
  const line = (start: number): LyricLine => ({ lrc_start: start, start, end: start + 1, text: "", syllables: [] });
  const lines = [line(1), line(3), line(5)];
  assert.equal(currentLine(lines, 0.5), -1);
  assert.equal(currentLine(lines, 1), 0);
  assert.equal(currentLine(lines, 4.9), 1);
  assert.equal(currentLine(lines, 99), 2);
  assert.equal(sungFraction(1, 2, 0.5), 0);
  assert.equal(sungFraction(1, 2, 1.25), 0.25);
  assert.equal(sungFraction(1, 2, 3), 1);
});

test("parses line times", () => {
  assert.equal(parseLineTime("01:12.13"), 72.13);
  assert.equal(parseLineTime("12.5"), 12.5);
  assert.equal(parseLineTime("1:75"), null);
  assert.equal(parseLineTime("soon"), null);
});
