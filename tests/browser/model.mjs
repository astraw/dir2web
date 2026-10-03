// Usage: node model.mjs <chromium|firefox|webkit> <model-view-page-url> [screenshot.png]
// Loads a 3D model view page (…/model.glb?view), waits for <model-viewer> to
// report the model loaded, checks that no request left the dir2web server
// (decoders must come from it, not a CDN), and optionally saves a screenshot.
// Headless Firefox has no WebGL on GPU-less machines; run it headed under a
// virtual display instead: HEADED=1 xvfb-run -a node model.mjs firefox URL
import { chromium, firefox, webkit } from "playwright";

const [which, url, shot] = process.argv.slice(2);
const browser = await ({ chromium, firefox, webkit }[which]).launch({
  headless: !process.env.HEADED,
  // Software WebGL for headless Chromium.
  args: which === "chromium" ? ["--enable-unsafe-swiftshader", "--use-angle=swiftshader"] : [],
});
const page = await browser.newPage({ viewport: { width: 1000, height: 800 } });
const origin = new URL(url).origin;
const requests = [];
const errors = [];
page.on("request", (r) => requests.push(r.url()));
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));

const t0 = Date.now();
await page.goto(url);
await page.waitForFunction(
  () => /^(Loaded|Could not|This browser)/.test(document.getElementById("status").textContent),
  null,
  { timeout: 60000 },
);
const status = await page.textContent("#status");
await page.waitForTimeout(1500); // let a few frames render
if (shot) await page.locator("model-viewer").screenshot({ path: shot });

const foreign = requests.filter((u) => !u.startsWith(origin) && !/^(data|blob):/.test(u));
const decoders = requests.filter((u) => /draco|basis/.test(u)).map((u) => new URL(u).pathname);
console.log(JSON.stringify({ which, secs: (Date.now() - t0) / 1000, status, decoders, foreign, errors }));
await browser.close();
process.exit(status.startsWith("Loaded") && foreign.length === 0 ? 0 : 1);
