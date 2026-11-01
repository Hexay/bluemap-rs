// usage: node crates/bm-wasm/bench/view.mjs <map url> <chrome exe> [--script unpack.js --wasm unpack.wasm | --no-script]
//   [--runs 3] [--seconds 30] [--net <latency ms>,<Mbit/s>] [--accept-encoding "gzip, deflate, br"] [--reload]
// Time to a rendered view in the real webapp, headless Chrome over the DevTools protocol: from navigation until the
// hires tile manager has loaded every tile it wants and none is loading, plus main-thread busy time and the longest
// task meanwhile. --script/--wasm replace the server's client script (A/B with one server); --no-script blanks it,
// so tiles come as server-made PRBM. Each run uses a fresh profile (cold HTTP cache). --reload then reloads the page
// in that profile and reports it under `reload`. `network` is what the browser's network stack saw once requests went
// quiet (bytes as on the wire, live/ polling excluded); its tile* fields are hires tiles only: `tileSpanMs` from the
// first request to the last finish, per tile `tileMs` request to finish, `tileBodyMs` response headers to finish.
// --dump <file> appends every request (netdump.mjs).
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { CLIENT_SCRIPT_PATTERN, closeBrowser, connect, fulfillScript, waitForTiles } from "./devtools.mjs";
import { netDump } from "./netdump.mjs";

const args = process.argv.slice(2);
const option = (name, fallback = null) => {
  const at = args.indexOf(name);
  return at >= 0 ? args.splice(at, ["--no-script", "--reload", "--no-gl-draw"].includes(name) ? 1 : 2)[1] ?? true : fallback;
};
const reload = option("--reload");
// skip GL draws: drops most of SwiftShader's cost, what is left is closer to a machine with a GPU
const noDraw = option("--no-gl-draw");
const scriptPath = option("--script");
const wasmPath = option("--wasm");
const noScript = option("--no-script");
const runs = Number(option("--runs", "3"));
const seconds = Number(option("--seconds", "30"));
// "<latency ms>,<download Mbit/s>": Network.emulateNetworkConditions
const net = option("--net")?.split(",").map(Number);
// replaces the browser's, e.g. "gzip, deflate, br" for a browser without zstd
const acceptEncoding = option("--accept-encoding");
const dumpFile = option("--dump");
const [url, browser] = args;
const replacement = noScript ? ""
  : scriptPath ? readFileSync(scriptPath, "utf8").replace("__UNPACK_WASM_BASE64__", readFileSync(wasmPath).toString("base64"))
  : null;

const middle = (xs) => (xs.length ? xs.toSorted((a, b) => a - b)[Math.floor(xs.length / 2)] : null);

// requests: Map of requestId -> what the Network events said about it
function networkSummary(requests) {
  const all = [...requests.values()];
  const tiles = all.filter((r) => /\/tiles\/0\//.test(r.url) && r.end && !r.cached);
  const statuses = {};
  for (const r of all) statuses[r.cached ? "cached" : r.status] = (statuses[r.cached ? "cached" : r.status] ?? 0) + 1;
  const sum = (rs) => rs.reduce((n, r) => n + (r.cached ? 0 : r.bytes ?? 0), 0);
  return {
    requests: all.length, statuses, bytes: sum(all), tileBytes: sum(tiles), tileRequests: tiles.length,
    tileSpanMs: tiles.length ? Math.round((Math.max(...tiles.map((r) => r.end)) - Math.min(...tiles.map((r) => r.start))) * 1e3) : null,
    tileMs: middle(tiles.map((r) => (r.end - r.start) * 1e3)),
    tileBodyMs: middle(tiles.filter((r) => r.headers).map((r) => (r.end - r.headers) * 1e3)),
    tileSumMs: Math.round(tiles.reduce((n, r) => n + (r.end - r.start) * 1e3, 0)),
    tileMaxMs: tiles.length ? Math.round(Math.max(...tiles.map((r) => r.end - r.start)) * 1e3) : null,
    tileReused: tiles.filter((r) => r.reused).length,
    // Nagle/delayed-ACK shows as ~40 ms between response headers and the end of the body
    tileBodyOver30: tiles.filter((r) => r.headers && r.end - r.headers > 0.03).length,
    tileUrls: tiles.map((r) => new URL(r.url).pathname),
  };
}

async function run() {
  const profile = mkdtempSync(join(tmpdir(), "bm-view-"));
  const child = spawn(browser, ["--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--enable-unsafe-swiftshader", "--window-size=1280,800", "--no-first-run",
    ...(noDraw ? ["--disable-gl-drawing-for-tests"] : []), "about:blank"], { stdio: "ignore" });
  try {
    const dump = dumpFile && netDump(dumpFile);
    const errors = [];
    let requests = new Map();
    let onNavigated = () => {};
    const track = (method, p) => {
      if (method === "Network.requestWillBeSent") {
        if (/^https?:/.test(p.request.url) && !p.request.url.includes("/live/") && p.type !== "EventSource")
          requests.set(p.requestId, { url: p.request.url, start: p.timestamp });
        return;
      }
      const r = requests.get(p.requestId);
      if (!r) return;
      if (method === "Network.responseReceived") {
        r.status = p.response.status;
        r.reused = p.response.connectionReused;
        if (p.response.fromDiskCache) r.cached = true;
        if (p.response.timing) r.headers = p.response.timing.requestTime + p.response.timing.receiveHeadersEnd / 1e3;
      } else if (method === "Network.requestServedFromCache") {
        r.cached = true;
      } else if (method === "Network.loadingFinished") {
        r.end = p.timestamp;
        r.bytes = p.encodedDataLength;
      } else if (method === "Network.loadingFailed") {
        r.end = p.timestamp;
        r.status = "failed";
      }
    };
    const send = await connect(profile, (method, params) => {
      if (method.startsWith("Network.")) {
        track(method, params);
        dump?.event(method, params);
      } else if (method === "Page.frameNavigated") {
        if (!params.frame.parentId) onNavigated();
      } else if (method === "Fetch.requestPaused") {
        fulfillScript(send, params.requestId, replacement);
      } else if (method === "Runtime.exceptionThrown") {
        errors.push(params.exceptionDetails.exception?.description ?? params.exceptionDetails.text);
      }
    });
    if (replacement !== null) await send("Fetch.enable", { patterns: [{ urlPattern: CLIENT_SCRIPT_PATTERN }] });
    await send("Runtime.enable");
    await send("Performance.enable");
    await send("Network.enable");
    await send("Page.enable");
    if (acceptEncoding) await send("Network.setExtraHTTPHeaders", { headers: { "Accept-Encoding": acceptEncoding } });
    if (net) {
      await send("Network.emulateNetworkConditions", { offline: false, latency: net[0],
        downloadThroughput: (net[1] * 1e6) / 8, uploadThroughput: (net[1] * 1e6) / 8 });
    }
    const task = async () =>
      (await send("Performance.getMetrics")).metrics.find((m) => m.name === "TaskDuration").value * 1e3;
    const measure = async (navigate) => {
      requests = new Map();
      const before = await task();
      // Page.reload answers before the old document is gone; WATCH on that one would finish at once
      const navigated = new Promise((resolve) => (onNavigated = resolve));
      await navigate();
      await navigated;
      const result = await waitForTiles(send, seconds);
      const after = await task();
      // gotcha: a reload restarts the page's TaskDuration from 0
      const mainMs = Math.round(after >= before ? after - before : after);
      // lowres and the rest may outlast the hires tiles
      for (let quiet = 0, i = 0; quiet < 5 && i < 300; i++) {
        quiet = [...requests.values()].every((r) => r.end) ? quiet + 1 : 0;
        await sleep(100);
      }
      return { ...result, mainMs, errors: errors.length, network: networkSummary(requests) };
    };
    const first = await measure(() => send("Page.navigate", { url }));
    dump?.startPhase("reload");
    const second = reload ? { reload: await measure(() => send("Page.reload")) } : {};
    dump?.flush();
    return { ...first, ...second };
  } finally {
    await closeBrowser(child, profile);
  }
}

const results = [];
for (let r = 0; r < runs; r++) results.push(await run());
const median = (k) => results.map((x) => x[k]).sort((a, b) => a - b)[Math.floor(results.length / 2)];
console.log(JSON.stringify({ script: noScript ? "none" : scriptPath ?? "server's", runs: results,
  median: { ms: Math.round(median("ms")), mainMs: median("mainMs"), longest: median("longest") } }));
// the browser's helper processes can keep handles open past its kill
process.exit(0);
