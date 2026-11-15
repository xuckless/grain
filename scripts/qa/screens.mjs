// Visual QA for the web build: screenshots of every view and popover at several window widths.
//
//   pnpm build:web
//   pnpm --filter web preview --port 4410 &      # serves models/weights at /models
//   node scripts/qa/screens.mjs http://localhost:4410 <song> <take> <outDir> [sizes=1024x700,1280x860,1600x1000]
//
// Runs offline (every non-localhost request is refused). With OVERLAY=1 the top bar gets the
// desktop's 88 px macOS window-button inset, so the desktop layout can be checked in a browser.
// Chrome's fake microphone stands in for a real one, so Karaoke can be driven: the Retake button,
// the calibrating popup, the take-saving card and the paused footer are captured too.
import { chromium } from "playwright-core";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const [url = "http://localhost:4410", song, take, outDir = "qa-shots", sizeArg = "1024x700,1280x860,1600x1000"] = process.argv.slice(2);
const sizes = sizeArg.split(",").map((s) => {
  const [w, h = "860"] = s.split("x");
  return { w: Number(w), h: Number(h) };
});
const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
mkdirSync(outDir, { recursive: true });

const browser = await chromium.launch({ executablePath: CHROME, headless: true, args: ["--autoplay-policy=no-user-gesture-required", "--enable-unsafe-webgpu", "--use-fake-ui-for-media-stream", "--use-fake-device-for-media-stream"] });
const page = await browser.newPage({ viewport: { width: sizes[0].w, height: sizes[0].h }, deviceScaleFactor: 1 });
await page.route(/^https?:\/\/(?!localhost|127\.0\.0\.1)/, (r) => r.abort());
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
if (process.env.OVERLAY) await page.addInitScript(() => {
  addEventListener("DOMContentLoaded", () => {
    const s = document.createElement("style");
    s.textContent = "header { padding-left: 88px !important }";
    document.head.append(s);
  });
});

const shot = async (name) => page.screenshot({ path: `${outDir}/${name}.png` });
const tab = (name) => page.getByRole("tab", { name, exact: true });
/** Opens a popover via its trigger, screenshots it, closes it with Escape or a second click. */
async function popover(trigger, name) {
  if (!(await trigger.count())) return;
  await trigger.first().click();
  await page.waitForTimeout(400);
  await shot(name);
  await page.keyboard.press("Escape");
  if ((await trigger.first().getAttribute("aria-expanded")) === "true") await trigger.first().click();
  await page.waitForTimeout(300);
}

try {
  await page.goto(url);
  await page.waitForTimeout(1500);
  await shot(`${sizes[0].w}-home`);
  const chooser = page.waitForEvent("filechooser");
  await page.getByRole("button", { name: "Open song…" }).click();
  await (await chooser).setFiles(resolve(song));
  await tab("Mix").waitFor({ timeout: 600_000 });
  await page.waitForTimeout(1500);
  if (take) {
    await tab("Score").click();
    const picker = page.waitForEvent("filechooser");
    await page.getByRole("button", { name: "More actions" }).click();
    await page.getByRole("menuitem", { name: /Import a take/ }).click();
    await (await picker).setFiles(resolve(take));
    await page.waitForTimeout(8000);
  }
  if (take) {
    // Karaoke: Retake beside the take picker, then Sing (calibrating popup), Pause (saving card),
    // the paused footer (Resume, Finish, Retake), and Finish.
    const { w, h } = sizes.at(-1);
    await page.setViewportSize({ width: w, height: h });
    const footer = page.locator("footer");
    await tab("Karaoke").click();
    await page.waitForTimeout(900);
    await shot(`${w}-karaoke-retake`);
    await footer.getByRole("button", { name: "Sing", exact: true }).click();
    await page.waitForTimeout(350);
    await shot(`${w}-karaoke-calibrating`);
    await footer.getByRole("button", { name: "Pause", exact: true }).waitFor({ timeout: 30_000 });
    await page.waitForTimeout(2500);
    await footer.getByRole("button", { name: "Pause", exact: true }).click();
    await page.waitForTimeout(150);
    await shot(`${w}-karaoke-saving`);
    await footer.getByRole("button", { name: "Resume", exact: true }).waitFor({ timeout: 30_000 });
    await page.waitForTimeout(1200);
    for (const s of sizes) {
      await page.setViewportSize({ width: s.w, height: s.h });
      await page.waitForTimeout(400);
      await shot(`${s.w}x${s.h}-karaoke-paused`);
      await page.mouse.wheel(0, 4000);
      await page.waitForTimeout(400);
      await shot(`${s.w}x${s.h}-karaoke-paused-scrolled`);
    }
    await footer.getByRole("button", { name: "Finish", exact: true }).click();
    await page.waitForTimeout(1000);
  }
  for (const { w, h } of sizes) {
    await page.setViewportSize({ width: w, height: h });
    await page.waitForTimeout(500);
    for (const view of ["Mix", "Score", "Karaoke", "Auto-tune", "Pitch", "Produce"]) {
      await tab(view).click();
      await page.waitForTimeout(900);
      await shot(`${w}-${view.toLowerCase()}`);
      await popover(page.getByRole("button", { name: "More actions" }), `${w}-${view.toLowerCase()}-more`);
    }
    await popover(page.getByRole("button", { name: "Settings", exact: true }), `${w}-settings`);
    await popover(page.locator("[data-export-trigger]"), `${w}-export`);
  }
} finally {
  if (errors.length) console.log("page errors:", errors.join(" | "));
  await browser.close();
}
