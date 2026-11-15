// Visual QA for the web build: screenshots of every view and popover at several window widths.
//
//   pnpm build:web
//   pnpm --filter web preview --port 4410 &      # serves models/weights at /models
//   node scripts/qa/screens.mjs http://localhost:4410 <song> <take> <outDir> [widths=1024,1280,1600]
//
// Runs offline (every non-localhost request is refused). With OVERLAY=1 the top bar gets the
// desktop's 88 px macOS window-button inset, so the desktop layout can be checked in a browser.
import { chromium } from "playwright-core";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";

const [url = "http://localhost:4410", song, take, outDir = "qa-shots", widthArg = "1024,1280,1600"] = process.argv.slice(2);
const widths = widthArg.split(",").map(Number);
const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
mkdirSync(outDir, { recursive: true });

const browser = await chromium.launch({ executablePath: CHROME, headless: true, args: ["--autoplay-policy=no-user-gesture-required", "--enable-unsafe-webgpu"] });
const page = await browser.newPage({ viewport: { width: widths[0], height: 860 }, deviceScaleFactor: 1 });
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
  await shot(`${widths[0]}-home`);
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
  for (const w of widths) {
    await page.setViewportSize({ width: w, height: 860 });
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
