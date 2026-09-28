// Usage: node play.mjs <chromium|firefox|webkit> <player-page-url>
// Plays a preview while it transcodes, seeks, and logs <video> state.
import { chromium, firefox, webkit } from "playwright";

const [which, url] = process.argv.slice(2);
const browser = await ({ chromium, firefox, webkit }[which]).launch({
  args: which === "chromium" ? ["--autoplay-policy=no-user-gesture-required"] : [],
  firefoxUserPrefs: { "media.autoplay.default": 0 },
});
const page = await browser.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));

const t0 = Date.now();
await page.goto(url);
const state = () =>
  page.evaluate(() => {
    const v = document.getElementById("v");
    const b = v.buffered;
    return {
      t: +v.currentTime.toFixed(2),
      dur: +v.duration.toFixed(1),
      paused: v.paused,
      muted: v.muted,
      err: v.error && v.error.message,
      buf: b.length ? [+b.start(0).toFixed(1), +b.end(b.length - 1).toFixed(1)] : null,
      hlsjs: typeof Hls !== "undefined",
      native: v.canPlayType("application/vnd.apple.mpegurl"),
      status: document.getElementById("status").textContent,
    };
  });
const log = async (label) =>
  console.log(`${which} +${((Date.now() - t0) / 1000).toFixed(2)}s ${label}`, JSON.stringify(await state()));

const avc = await page.evaluate(() =>
  typeof MediaSource !== "undefined" && MediaSource.isTypeSupported('video/mp4; codecs="avc1.64001f,mp4a.40.2"'));
console.log(`${which} MSE H.264+AAC supported: ${avc}`);

await page.waitForFunction(() => document.getElementById("v").currentTime > 0.2, null, { timeout: 30000 });
await log("playing");
await page.waitForTimeout(3000);
await log("after 3 s");

// Seek backwards within what is already transcoded.
await page.evaluate(() => { document.getElementById("v").currentTime = 1; });
await page.waitForTimeout(1500);
await log("after seek to 1 s");

// Wait for the transcode to finish, then jump near the end.
await page.waitForFunction(() => /complete/.test(document.getElementById("status").textContent), null, { timeout: 120000 });
await page.waitForTimeout(2500); // let the player reload the final playlist
await log("transcode done");
await page.evaluate(() => { const v = document.getElementById("v"); v.currentTime = 200; });
await page.waitForTimeout(2500);
await log("after seek to 200 s");
console.log(`${which} page errors: ${JSON.stringify(errors)}`);
await browser.close();
