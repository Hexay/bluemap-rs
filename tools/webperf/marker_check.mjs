// usage: node tools/webperf/marker_check.mjs <map url> <out dir> [--clicks 5] [--cpus 2] [--max-rss 2500]
// Shape-marker batching (docs/18 "Many shape markers") against upstream drawing, in a capped Chrome (box.mjs) per
// variant: a screenshot of the overview and of a close-up, then clicks on the centres of shape markers no other shape
// overlaps. A click passes when the label popup that opens shows that marker's label/detail. One line per variant.
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
import { connect } from "../../crates/bm-wasm/bench/devtools.mjs";
import { installTeardown, launch, preflight, QUIET_CHROME_FLAGS, waitForTurn } from "./box.mjs";

const args = process.argv.slice(2);
const option = (name, fallback) => {
  const at = args.indexOf(name);
  return at >= 0 ? args.splice(at, 2)[1] : fallback;
};
const clicks = Number(option("--clicks", "5"));
const cpus = Number(option("--cpus", "2"));
const maxRss = Number(option("--max-rss", "2500"));
const chrome = option("--chrome", join(homedir(), ".cache/ms-playwright/chromium-1248/chrome-linux64/chrome"));
const [url, outDir] = args;
if (!outDir) throw new Error("usage: marker_check.mjs <map url> <out dir>");
const log = (m) => process.stderr.write(`${m}\n`);
const VARIANTS = { upstream: { markerBatch: 0, markerPoll: 0 }, batched: {} };
const W = 960, H = 600;

// in the page: resolves once a marker set holds shape markers (and, batched, its batch is built)
const READY = (batched) => `new Promise((resolve) => {
  const t0 = performance.now();
  const poll = () => {
    const shapes = [];
    window.bluemap?.mapViewer?.markers?.traverse((o) => o.isShapeMarker && shapes.push(o));
    let batch = 0;
    window.bluemap?.mapViewer?.markers?.traverse((o) => (o.isMesh || o.isLine2) && !o.parent?.isShapeMarker && batch++);
    if ((shapes.length && (!${batched} || batch)) || performance.now() - t0 > 90000)
      return setTimeout(() => resolve({ shapes: shapes.length, batch }), 3000);
    setTimeout(poll, 250);
  };
  poll();
})`;

// in the page: move close to the i-th shape marker (by id) whose centre no other shape covers, wait until the camera
// stops moving, return its screen centre and expected popup text
const AIM = (i) => `new Promise((resolve) => {
  const v = window.bluemap.mapViewer, shapes = [];
  v.markers.traverse((o) => o.isShapeMarker && o.visible && shapes.push(o));
  shapes.sort((a, b) => (a.data.id < b.data.id ? -1 : 1));
  const V3 = shapes[0].position.constructor;
  const boxes = shapes.map((m) => {
    m.fill.geometry.computeBoundingBox();
    return m.fill.geometry.boundingBox.clone().applyMatrix4(m.fill.matrixWorld);
  });
  const free = window.__bmFree ??= shapes.map((m, k) => k).filter((k) => {
    const c = boxes[k].getCenter(new V3());
    return boxes.every((b, j) => j === k || c.x < b.min.x || c.x > b.max.x || c.z < b.min.z || c.z > b.max.z);
  });
  const k = free[Math.floor((${i} + 0.5) * free.length / ${clicks})], m = shapes[k];
  const c = boxes[k].getCenter(new V3());
  const cm = v.controlsManager;
  cm.position.x = c.x; cm.position.z = c.z; cm.distance = 120; cm.angle = 0; cm.rotation = 0;
  let last = null, still = 0;
  const t0 = performance.now();
  const settle = () => {
    const now = v.camera.position.toArray().map((x) => x.toFixed(2)).join();
    still = now === last ? still + 1 : 0;
    last = now;
    if (still < 3 && performance.now() - t0 < 20000) return requestAnimationFrame(settle);
    const p = c.clone().project(v.camera);
    document.querySelectorAll(".bm-marker-labelpopup").forEach((e) => e.remove());
    resolve({ id: m.data.id, free: free.length, x: (p.x + 1) / 2 * innerWidth, y: (1 - p.y) / 2 * innerHeight,
      text: (m.data.detail || m.data.label || "").replace(/<[^>]*>/g, "").trim() });
  };
  requestAnimationFrame(settle);
})`;
// waits up to 10 s for a label popup; their texts
const POPUP = `new Promise((resolve) => {
  const t0 = performance.now();
  const poll = () => {
    const texts = [...document.querySelectorAll(".bm-marker-labelpopup")].map((e) => e.textContent.trim());
    if (texts.length || performance.now() - t0 > 10000) return resolve(texts);
    setTimeout(poll, 100);
  };
  poll();
})`;

async function variant(name, tuning) {
  preflight(maxRss);
  const profile = mkdtempSync(join(tmpdir(), "bm-webperf-"));
  const browser = launch(chrome, ["--headless=new", "--no-sandbox", "--remote-debugging-port=0",
    `--user-data-dir=${profile}`, "--enable-unsafe-swiftshader", `--window-size=${W},${H}`, "--no-first-run",
    ...QUIET_CHROME_FLAGS, "about:blank"], { cpus, maxRssMB: maxRss, maxSeconds: 300, tag: profile, log });
  try {
    const errors = [];
    const send = await connect(profile, (m, p) => m === "Runtime.exceptionThrown"
      && errors.push(p.exceptionDetails.exception?.description ?? p.exceptionDetails.text));
    const evaluate = async (expression) =>
      (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })).result.value;
    const shot = async (file) => writeFileSync(join(outDir, file),
      Buffer.from((await send("Page.captureScreenshot", { format: "png" })).data, "base64"));
    await send("Page.enable");
    await send("Runtime.enable");
    await send("Page.addScriptToEvaluateOnNewDocument",
      { source: `try { localStorage.setItem("bluemap-rs-client-tuning", ${JSON.stringify(JSON.stringify(tuning))}); } catch {}` });
    await send("Page.navigate", { url });
    let ready = null;
    for (let i = 0; i < 100 && !ready; i++) ready = await evaluate(READY(name === "batched")).catch(() => null);
    await shot(`${name}-overview.png`);
    let hit = 0;
    const misses = [];
    for (let i = 0; i < clicks; i++) {
      const aim = await evaluate(AIM(i));
      if (i === 0) await shot(`${name}-closeup.png`);
      // what a tap calls; a real tap is unreliable here: Hammer's tap needs press→release within 250 ms
      await evaluate(`window.bluemap.mapViewer.handleMapInteraction({ x: ${aim.x}, y: ${aim.y} }, {})`);
      const popups = await evaluate(POPUP);
      if (popups.some((p) => p === aim.text)) hit++;
      else misses.push({ id: aim.id, want: aim.text.slice(0, 40), got: popups.map((p) => p.slice(0, 40)) });
      if (i === 0) await shot(`${name}-click.png`);
    }
    console.log(JSON.stringify({ variant: name, ...ready, clicks: `${hit}/${clicks}`, misses, pageErrors: errors.length }));
  } finally {
    browser.stop();
    rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}

installTeardown(log);
if (!(await waitForTurn(10 * 60e3, log))) throw new Error("another webperf run stayed live for 10 min");
for (const [name, tuning] of Object.entries(VARIANTS)) await variant(name, tuning);
process.exit(0);
