// usage: node crates/bm-wasm/bench/gpu_trace.mjs <map url> <chrome exe> [--net <latency ms>,<Mbit/s>] [--no-script]
//   [--out <path prefix>] [--seconds 60]
// A headed Chrome on the real GPU, fresh profile: a Chrome trace of the first load and of a reload of one view, each
// from navigation until the hires tile manager has its tiles, saved as <prefix>-first.json and <prefix>-reload.json
// (chrome://tracing) and summarized by trace_summary.mjs. Keep the window visible: an occluded one may not draw.
import { spawn } from "node:child_process";
import { createWriteStream, mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { once } from "node:events";
import { CLIENT_SCRIPT_PATTERN, closeBrowser, connect, fulfillScript, waitForTiles } from "./devtools.mjs";
import { printSummary, summarize } from "./trace_summary.mjs";
import { readFileSync } from "node:fs";

const args = process.argv.slice(2);
const flag = (name) => {
  const at = args.indexOf(name);
  return at >= 0 ? !!args.splice(at, 1) : false;
};
const option = (name, fallback = null) => {
  const at = args.indexOf(name);
  return at >= 0 ? args.splice(at, 2)[1] : fallback;
};
const noScript = flag("--no-script");
const net = option("--net")?.split(",").map(Number);
const seconds = Number(option("--seconds", "60"));
const prefix = resolve(option("--out", join(tmpdir(), `bm-gpu-trace-${Date.now()}`)));
const [url, browser] = args;
if (!url || !browser) throw new Error("usage: gpu_trace.mjs <map url> <chrome exe> [--net l,m] [--no-script] [--out prefix]");

const CATEGORIES = ["toplevel", "devtools.timeline", "disabled-by-default-devtools.timeline", "v8", "v8.execute",
  "disabled-by-default-v8.compile", "disabled-by-default-v8.cpu_profiler", "blink.user_timing", "loading", "gpu", "viz"];
const RENDERER = `(() => {
  const gl = document.createElement("canvas").getContext("webgl2");
  const info = gl?.getExtension("WEBGL_debug_renderer_info");
  return info ? gl.getParameter(info.UNMASKED_RENDERER_WEBGL) : "no WEBGL_debug_renderer_info";
})()`;

async function saveTrace(send, stream, path) {
  const file = createWriteStream(path);
  for (let eof = false; !eof;) {
    const chunk = await send("IO.read", { handle: stream, size: 4 << 20 });
    if (!file.write(Buffer.from(chunk.data, chunk.base64Encoded ? "base64" : "utf8"))) await once(file, "drain");
    eof = chunk.eof;
  }
  file.end();
  await once(file, "finish");
  await send("IO.close", { handle: stream });
}

const profile = mkdtempSync(join(tmpdir(), "bm-gpu-"));
const child = spawn(browser, ["--remote-debugging-port=0", `--user-data-dir=${profile}`, "--window-size=1280,800",
  "--no-first-run", "--no-default-browser-check", "--disable-extensions", "--disable-background-timer-throttling",
  "--disable-renderer-backgrounding", "--disable-backgrounding-occluded-windows",
  "--disable-features=CalculateNativeWinOcclusion", "about:blank"], { stdio: "ignore" });
let onNavigated = () => {};
let onTraced = () => {};
try {
  const send = await connect(profile, (method, params) => {
    if (method === "Page.frameNavigated" && !params.frame.parentId) onNavigated();
    else if (method === "Tracing.tracingComplete") onTraced(params.stream);
    else if (method === "Fetch.requestPaused") fulfillScript(send, params.requestId, "");
  });
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Page.bringToFront");
  if (noScript) await send("Fetch.enable", { patterns: [{ urlPattern: CLIENT_SCRIPT_PATTERN }] });
  if (net) {
    await send("Network.enable");
    await send("Network.emulateNetworkConditions", { offline: false, latency: net[0],
      downloadThroughput: (net[1] * 1e6) / 8, uploadThroughput: (net[1] * 1e6) / 8 });
  }
  const renderer = (await send("Runtime.evaluate", { expression: RENDERER })).result.value;
  console.log(`renderer: ${renderer}`);
  if (/swiftshader|llvmpipe|basic render/i.test(renderer)) console.log("WARNING: a software renderer, not the GPU");

  const trace = async (name, navigate) => {
    await send("Tracing.start", { transferMode: "ReturnAsStream", traceConfig: { recordMode: "recordAsMuchAsPossible",
      includedCategories: CATEGORIES } });
    // Page.reload answers before the old document is gone; watching that one would finish at once
    const navigated = new Promise((done) => (onNavigated = done));
    await navigate();
    await navigated;
    const view = await waitForTiles(send, seconds);
    const traced = new Promise((done) => (onTraced = done));
    await send("Tracing.end");
    const path = `${prefix}-${name}.json`;
    await saveTrace(send, await traced, path);
    printSummary(name, view, summarize(JSON.parse(readFileSync(path, "utf8"))), path);
  };
  await trace("first", () => send("Page.navigate", { url }));
  await trace("reload", () => send("Page.reload"));
} finally {
  await closeBrowser(child, profile);
}
// the browser's helper processes can keep handles open past its kill
process.exit(0);
