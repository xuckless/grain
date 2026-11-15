// UI frame-time harness for the web build (AGENTS.md §10.1).
//
//   pnpm build:web
//   pnpm --filter web preview --port 4400 &      # serves models/weights at /models
//   node scripts/perf/run.mjs http://localhost:4400 samples/eill-finale.mp3 [out.json]
//
// Imports the song in headless Chrome with the frame-rate limit off, so each frame's time is
// the work it took (not the display's vsync), then runs the scenarios below and prints
// frame-time p50/p95/max, long tasks (> 50 ms) and main-thread → audio-thread messages per
// second (the web build's stand-in for desktop IPC). Env: CHROME (browser path), DPR (device
// pixel ratio, default 2), SOFTWARE=1 (software raster).
import { chromium } from "playwright-core";
import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

const [url = "http://localhost:4400", song = "samples/eill-finale.mp3", out] = process.argv.slice(2);
const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";

const browser = await chromium.launch({
  executablePath: CHROME,
  headless: true,
  // GPU raster (as in the app); SOFTWARE=1 measures with the software rasteriser instead.
  args: [
    "--disable-frame-rate-limit",
    "--disable-gpu-vsync",
    "--autoplay-policy=no-user-gesture-required",
    "--enable-unsafe-webgpu",
    ...(process.env.SOFTWARE ? [] : ["--enable-gpu", "--ignore-gpu-blocklist", "--use-angle=metal"]),
  ],
});
// DPR=1 or 2 (default 2: a Retina display, where canvases cost 4× the pixels).
const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: Number(process.env.DPR ?? 2) });

// Frame sampler: every rAF delta, long tasks and posted messages go into the open bucket.
await page.addInitScript(() => {
  const state = { bucket: null };
  const post = MessagePort.prototype.postMessage;
  MessagePort.prototype.postMessage = function (...a) {
    if (state.bucket) state.bucket.messages++;
    return post.apply(this, a);
  };
  let last = 0;
  const frame = (t) => {
    if (state.bucket && last) state.bucket.frames.push(t - last);
    last = t;
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);
  new PerformanceObserver((l) => {
    for (const e of l.getEntries()) if (state.bucket) state.bucket.longTasks.push(e.duration);
  }).observe({ entryTypes: ["longtask"] });
  window.__perf = {
    begin() {
      state.bucket = { frames: [], longTasks: [], messages: 0, t0: performance.now() };
    },
    end() {
      const b = state.bucket;
      state.bucket = null;
      const f = [...b.frames].sort((x, y) => x - y);
      const q = (p) => (f.length ? f[Math.min(f.length - 1, Math.floor(p * f.length))] : 0);
      const secs = (performance.now() - b.t0) / 1000;
      return {
        frames: f.length,
        fps: +(f.length / secs).toFixed(0),
        p50: +q(0.5).toFixed(1),
        p95: +q(0.95).toFixed(1),
        // Uncapped, most frames have no input to handle: p99 is the frames that did.
        p99: +q(0.99).toFixed(1),
        max: +(f.at(-1) ?? 0).toFixed(1),
        longTasks: b.longTasks.length,
        longestTask: +Math.max(0, ...b.longTasks).toFixed(0),
        messagesPerSec: +(b.messages / secs).toFixed(0),
      };
    },
  };
});
// The harness stays offline: Karaoke's lyrics lookup (LRCLIB) is refused.
await page.route(/^https?:\/\/(?!localhost|127\.0\.0\.1)/, (r) => r.abort());
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));

const results = {};
async function measure(name, action, ms) {
  await page.evaluate(() => window.__perf.begin());
  await action();
  if (ms) await page.waitForTimeout(ms);
  results[name] = await page.evaluate(() => window.__perf.end());
  console.log(name.padEnd(22), JSON.stringify(results[name]));
}

/** Drags the pointer back and forth across `box` for `ms`, one move every ~8 ms. */
async function sweep(box, ms) {
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + 12, y);
  await page.mouse.down();
  const t0 = Date.now();
  let i = 0;
  while (Date.now() - t0 < ms) {
    const f = (Math.sin(i++ / 12) + 1) / 2;
    await page.mouse.move(box.x + 4 + f * (box.width - 8), y);
    await page.waitForTimeout(8);
  }
  await page.mouse.up();
}

/** Pauses the song if it's playing. */
async function pause() {
  const button = page.getByRole("button", { name: "Pause", exact: true });
  if (await button.count()) await button.click();
}

try {
  await page.goto(url);
  await page.waitForTimeout(1500);
  await measure("home idle", async () => {}, 3000);

  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Open song…" }).click();
  await (await chooser).setFiles(resolve(song));
  await page.getByRole("tab", { name: "Mix" }).waitFor({ timeout: 600_000 });
  await page.waitForTimeout(2000);

  await measure("mix idle", async () => {}, 3000);
  const gain = await page.locator("input[aria-label$=' gain']").first().evaluate((el) => {
    const r = el.closest(".slider").getBoundingClientRect();
    return { x: r.x, y: r.y, width: r.width, height: r.height };
  });
  await measure("mix gain drag", () => sweep(gain, 4000), 0);
  const play = page.getByRole("button", { name: "Play", exact: true });
  await measure("mix playback", () => play.click(), 4000);
  await measure("mix drag while playing", () => sweep(gain, 3000), 0);
  const lane = await page.locator(".lane").nth(2).boundingBox();
  await measure("mix scrub", () => sweep(lane, 3000), 0);
  await pause();

  const tabs = ["Score", "Karaoke", "Auto-tune", "Pitch", "Produce", "Mix"];
  await measure("tab switching", async () => {
    for (const t of tabs) {
      await page.getByRole("tab", { name: t }).click();
      await page.waitForTimeout(500);
    }
  }, 0);
  for (const t of tabs.slice(0, 5)) {
    await page.getByRole("tab", { name: t }).click();
    await page.waitForTimeout(800);
    await measure(`${t} idle`, async () => {}, 2000);
  }
  await page.getByRole("tab", { name: "Pitch" }).click();
  await page.waitForTimeout(800);
  await measure("Pitch playback", () => page.getByRole("button", { name: "Play", exact: true }).click(), 3000);
  await pause();
  const roll = await page.locator(".roll").boundingBox();
  const middle = { x: roll.x + roll.width * 0.3, y: roll.y, width: roll.width * 0.4, height: roll.height };
  await measure("pitch editor drag", () => sweep(middle, 3000), 0);
} finally {
  if (errors.length) console.log("page errors:", errors.join(" | "));
  if (out) writeFileSync(out, JSON.stringify(results, null, 2));
  await browser.close();
}
