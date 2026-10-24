// usage: node crates/bm-wasm/bench/view.mjs <map url> <chrome exe> [--script unpack.js --wasm unpack.wasm | --no-script]
//   [--runs 3] [--seconds 30] [--net <latency ms>,<Mbit/s>]
// Time to a rendered view in the real webapp, headless Chrome over the DevTools protocol: from navigation until the
// hires tile manager has loaded every tile it wants and none is loading, plus main-thread busy time and the longest
// task meanwhile. --script/--wasm replace the server's client script (A/B with one server); --no-script blanks it,
// so tiles come as server-made PRBM. Each run uses a fresh profile (cold HTTP cache).
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { pageSocket } from "./devtools.mjs";

const args = process.argv.slice(2);
const option = (name, fallback = null) => {
  const at = args.indexOf(name);
  return at >= 0 ? args.splice(at, name === "--no-script" ? 1 : 2)[1] ?? true : fallback;
};
const scriptPath = option("--script");
const wasmPath = option("--wasm");
const noScript = option("--no-script");
const runs = Number(option("--runs", "3"));
const seconds = Number(option("--seconds", "30"));
// "<latency ms>,<download Mbit/s>": Network.emulateNetworkConditions
const net = option("--net")?.split(",").map(Number);
const [url, browser] = args;
const replacement = noScript ? ""
  : scriptPath ? readFileSync(scriptPath, "utf8").replace("__UNPACK_WASM_BASE64__", readFileSync(wasmPath).toString("base64"))
  : null;

// in the page: resolves once the hires tiles of the view are in
const WATCH = `new Promise((resolve) => {
  const started = performance.now();
  let longest = 0;
  new PerformanceObserver((list) => list.getEntries().forEach((e) => (longest = Math.max(longest, e.duration))))
    .observe({ type: "longtask", buffered: true });
  let stable = 0;
  const poll = () => {
    const m = window.bluemap?.mapViewer?.map?.hiresTileManager;
    const tiles = m ? [...m.tiles.values()] : [];
    const loaded = tiles.filter((t) => t.model).length;
    const done = tiles.length > 0 && loaded === tiles.length && m.currentlyLoading === 0;
    stable = done ? stable + 1 : 0;
    if (stable >= 3 || performance.now() - started > ${seconds * 1000})
      return resolve(JSON.stringify({ ms: performance.now(), loaded, wanted: tiles.length, longest,
        loading: tiles.filter((t) => t.loading).length, unloaded: tiles.filter((t) => t.unloaded).length }));
    setTimeout(poll, 20);
  };
  poll();
})`;

async function run() {
  const profile = mkdtempSync(join(tmpdir(), "bm-view-"));
  const child = spawn(browser, ["--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
    "--enable-unsafe-swiftshader", "--window-size=1280,800", "--no-first-run", "about:blank"], { stdio: "ignore" });
  try {
    const socket = new WebSocket(await pageSocket(profile));
    await new Promise((resolve, reject) => ((socket.onopen = resolve), (socket.onerror = reject)));
    let nextId = 0;
    const pending = new Map();
    const send = (method, params = {}) =>
      new Promise((resolve, reject) => {
        pending.set(++nextId, { resolve, reject });
        socket.send(JSON.stringify({ id: nextId, method, params }));
      });
    const errors = [];
    socket.onmessage = ({ data }) => {
      const m = JSON.parse(data);
      if (m.id) {
        const { resolve, reject } = pending.get(m.id);
        pending.delete(m.id);
        return m.error ? reject(new Error(m.error.message)) : resolve(m.result);
      }
      if (m.method === "Fetch.requestPaused") {
        send("Fetch.fulfillRequest", { requestId: m.params.requestId, responseCode: 200,
          responseHeaders: [{ name: "Content-Type", value: "text/javascript" }],
          body: Buffer.from(replacement).toString("base64") });
      } else if (m.method === "Runtime.exceptionThrown") {
        errors.push(m.params.exceptionDetails.exception?.description ?? m.params.exceptionDetails.text);
      }
    };
    if (replacement !== null) await send("Fetch.enable", { patterns: [{ urlPattern: "*bluemap-rs-unpack-*" }] });
    await send("Runtime.enable");
    await send("Performance.enable");
    if (net) {
      await send("Network.enable");
      await send("Network.emulateNetworkConditions", { offline: false, latency: net[0],
        downloadThroughput: (net[1] * 1e6) / 8, uploadThroughput: (net[1] * 1e6) / 8 });
    }
    const task = async () =>
      (await send("Performance.getMetrics")).metrics.find((m) => m.name === "TaskDuration").value * 1e3;
    const before = await task();
    await send("Page.navigate", { url });
    let result;
    for (let i = 0; i < 100 && !result; i++) {
      try {
        const r = await send("Runtime.evaluate", { expression: WATCH, awaitPromise: true });
        result = r.result?.value && JSON.parse(r.result.value);
      } catch {
        await sleep(100); // context replaced while navigating
      }
    }
    return { ...result, mainMs: Math.round((await task()) - before), errors: errors.length };
  } finally {
    child.kill();
    await sleep(800);
    try {
      rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 300 });
    } catch {
      // helper processes may hold the profile a moment longer; it is a temp dir
    }
  }
}

const results = [];
for (let r = 0; r < runs; r++) results.push(await run());
const median = (k) => results.map((x) => x[k]).sort((a, b) => a - b)[Math.floor(results.length / 2)];
console.log(JSON.stringify({ script: noScript ? "none" : scriptPath ?? "server's", runs: results,
  median: { ms: Math.round(median("ms")), mainMs: median("mainMs"), longest: median("longest") } }));
// the browser's helper processes can keep handles open past its kill
process.exit(0);
