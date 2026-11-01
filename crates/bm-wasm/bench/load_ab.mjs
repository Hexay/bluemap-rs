// usage: node crates/bm-wasm/bench/load_ab.mjs <map url> <chrome> --label <row> --out rows.jsonl [--net <ms>,<Mbit/s>]
//   [--script unpack.js --wasm unpack.wasm] [--trace] [--no-pan] [--seconds 90] [--run n]
// One run in headless Chrome, GL draws off, fresh profile: first load, pan, reload, revisit (a navigation to the same URL
// in the same profile). Each phase: time until every hires tile the manager wants has its model, time until the on-screen
// ones have, network per category, slow frames (main thread only: no GPU here), holes. --trace adds a wall-clock partition
// of the reload (load_parts.mjs); tracing slows the page, so do not compare traced times with untraced ones. --script
// swaps the server's client script, so one server serves both sides of an A/B.
import { spawn } from "node:child_process";
import { appendFileSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { CLIENT_SCRIPT_PATTERN, assembleScript, closeBrowser, connect, fulfillScript, waitForTiles } from "./devtools.mjs";
import { milestones, partition, requestStats } from "./load_parts.mjs";
import { summarize } from "./trace_summary.mjs";
import { ONSCREEN_RECORDER, holes, onscreenResult } from "./load_page.mjs";
import { NOW, RECORDER, pan, slice, teleport } from "./smoothness_page.mjs";
import { frameStats, latencyStats } from "./smoothness_stats.mjs";

const args = process.argv.slice(2);
const option = (name, fallback = null) => {
  const at = args.indexOf(name);
  return at >= 0 ? args.splice(at, 2)[1] : fallback;
};
const flag = (name) => {
  const at = args.indexOf(name);
  return at >= 0 ? !!args.splice(at, 1) : false;
};
const label = option("--label", "row");
const out = option("--out");
const net = option("--net")?.split(",").map(Number);
const scriptPath = option("--script");
const wasmPath = option("--wasm");
const seconds = Number(option("--seconds", "90"));
const runIndex = Number(option("--run", "1"));
const traceReload = flag("--trace");
const noPan = flag("--no-pan");
// SwiftShader draws: uploads and draws on the CPU, a weak stand-in for a GPU
const glDraw = flag("--gl-draw");
const [url, browser] = args;
if (!url || !browser) throw new Error("usage: load_ab.mjs <map url> <chrome> --label x --out rows.jsonl");
const replacement = scriptPath ? assembleScript(scriptPath, wasmPath) : null;
// e.g. '{"pace":4}': the client script's features for this row
const tune = option("--tune");
// --zigzag: reverses every 2.5 s, the worst case for guessing the direction of travel
const PATH = flag("--zigzag")
  ? [[0, 0], [150, 0], [0, 0], [150, 0], [0, 0], [150, 0]].map(([x, z], i) => [x + i * 30, z + i * 60])
  : [[0, 0], [360, 0], [360, 360]];
const PAN_MS = 12000;
const deadline = (promise, ms, what) => Promise.race([promise,
  sleep(ms, null, { ref: false }).then(() => { throw new Error(`${what} missed its ${ms} ms deadline`); })]);

const profile = mkdtempSync(join(tmpdir(), "bm-load-"));
const child = spawn(browser, ["--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
  "--enable-unsafe-swiftshader", "--window-size=1280,800", "--no-first-run", ...(glDraw ? [] : ["--disable-gl-drawing-for-tests"]),
  "about:blank"], { stdio: "ignore" });
const row = { label, net: net?.join(",") ?? "lo", run: runIndex, at: new Date().toISOString(), phases: {} };
try {
  let requests = new Map();
  let onNavigated = () => {};
  let onTraced = () => {};
  const errors = [];
  const track = (method, p) => {
    if (method === "Network.requestWillBeSent") {
      if (/^https?:/.test(p.request.url) && !p.request.url.includes("/live/") && p.type !== "EventSource")
        requests.set(p.requestId, { url: p.request.url, type: p.type, start: p.timestamp, conditional: !!p.request.headers["If-None-Match"] });
      return;
    }
    const r = requests.get(p.requestId);
    if (!r) return;
    if (method === "Network.requestWillBeSentExtraInfo") {
      if (Object.keys(p.headers).some((k) => k.toLowerCase() === "if-none-match")) r.conditional = true;
    } else if (method === "Network.responseReceived") {
      r.status = p.response.status;
      if (p.response.fromDiskCache) r.cached = true;
      const t = p.response.timing;
      if (t) {
        r.sendAt = t.requestTime + t.sendStart / 1e3;
        r.headersAt = t.requestTime + t.receiveHeadersEnd / 1e3;
      }
    } else if (method === "Network.requestServedFromCache") {
      r.cached = true;
    } else if (method === "Network.loadingFinished") {
      r.end = p.timestamp;
      r.bytes = p.encodedDataLength;
    } else if (method === "Network.loadingFailed") {
      r.end = p.timestamp;
      r.failed = p.errorText;
    }
  };
  const send = await connect(profile, (method, params) => {
    if (method.startsWith("Network.")) track(method, params);
    else if (method === "Page.frameNavigated" && !params.frame.parentId) onNavigated();
    else if (method === "Fetch.requestPaused") fulfillScript(send, params.requestId, replacement);
    else if (method === "Tracing.tracingComplete") onTraced(params.stream);
    else if (method === "Runtime.exceptionThrown") errors.push(params.exceptionDetails.exception?.description ?? params.exceptionDetails.text);
  });
  const evaluate = async (expression, awaitPromise = false) => {
    const r = await send("Runtime.evaluate", { expression, awaitPromise });
    if (r.exceptionDetails) throw new Error(`page: ${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`);
    return r.result.value;
  };
  if (replacement !== null) await send("Fetch.enable", { patterns: [{ urlPattern: CLIENT_SCRIPT_PATTERN }] });
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Performance.enable");
  await send("Page.addScriptToEvaluateOnNewDocument", { source: RECORDER });
  await send("Page.addScriptToEvaluateOnNewDocument", { source: ONSCREEN_RECORDER });
  if (tune) await send("Page.addScriptToEvaluateOnNewDocument", { source: `try { localStorage.setItem("bluemap-rs-client-tuning", ${JSON.stringify(tune)}); } catch {}` });
  await send("Network.enable");
  if (net) {
    await send("Network.emulateNetworkConditions", { offline: false, latency: net[0],
      downloadThroughput: (net[1] * 1e6) / 8, uploadThroughput: (net[1] * 1e6) / 8 });
  }
  const quiet = async () => {
    for (let q = 0, i = 0; q < 5 && i < 1500; i++) {
      q = [...requests.values()].every((r) => r.end) ? q + 1 : 0;
      await sleep(100);
    }
  };
  const task = async () => (await send("Performance.getMetrics")).metrics.find((m) => m.name === "TaskDuration").value * 1e3;
  const stats = async (t0, t1) => JSON.parse(await evaluate(slice(t0, t1)));
  const frames = (seen) => {
    const f = frameStats(seen.intervals, seen.tasks);
    return { over33: seen.intervals.filter((x) => x > 33.4).length, over50: seen.intervals.filter((x) => x > 50).length,
      frames: seen.intervals.length, p50: f.p50, longest: seen.tasks.length ? Math.round(Math.max(...seen.tasks)) : 0 };
  };

  const load = async (name, navigate, trace = false) => {
    requests = new Map();
    if (trace) {
      await send("Tracing.start", { transferMode: "ReturnAsStream", traceConfig: { recordMode: "recordAsMuchAsPossible",
        includedCategories: ["toplevel", "devtools.timeline", "disabled-by-default-devtools.timeline", "v8", "v8.execute",
          "disabled-by-default-v8.compile", "disabled-by-default-v8.cpu_profiler", "blink.user_timing", "loading", "gpu", "viz"] } });
    }
    const navigated = new Promise((done) => (onNavigated = done));
    await navigate();
    await deadline(navigated, 30000, `${name} navigation`);
    const before = name === "first" ? await task() : 0;
    const view = await waitForTiles(send, seconds);
    const main = Math.round((await task()) - before);
    const onscreen = JSON.parse(await evaluate(onscreenResult));
    const frameSeen = frames(await stats(0, view.ms));
    const inView = [...requests.values()];
    const t0 = inView.filter((r) => r.type === "Document").map((r) => r.start)[0] ?? Math.min(...inView.map((r) => r.start));
    const t1 = t0 + view.ms / 1e3;
    const atView = requestStats(inView.filter((r) => r.start <= t1));
    await quiet();
    const all = [...requests.values()];
    const phase = { ms: Math.round(view.ms), loaded: view.loaded, wanted: view.wanted, ...onscreen, mainMs: main, frames: frameSeen,
      milestones: milestones(all, t0), atView, total: requestStats(all), holes: JSON.parse(await evaluate(holes)) };
    if (trace) {
      const traced = new Promise((done) => (onTraced = done));
      await send("Tracing.end");
      const stream = await deadline(traced, 60000, "trace");
      let text = "";
      for (let eof = false; !eof;) {
        const c = await send("IO.read", { handle: stream, size: 4 << 20 });
        text += c.base64Encoded ? Buffer.from(c.data, "base64").toString() : c.data;
        eof = c.eof;
      }
      await send("IO.close", { handle: stream });
      const events = JSON.parse(text).traceEvents;
      phase.partition = partition(events, all, t0, t1);
      const t0us = t0 * 1e6, t1us = t1 * 1e6;
      const s = summarize({ traceEvents: events.filter((e) => e.ph === "M" || (e.ts >= t0us && e.ts <= t1us)) });
      phase.mainSplit = s.split;
      phase.mainLongest = s.longest;
    }
    row.phases[name] = phase;
  };

  await load("first", () => send("Page.navigate", { url }));
  if (!noPan) {
    const t = await evaluate(NOW);
    const before = await task();
    requests = new Map();
    const { latencies, ...r } = JSON.parse(await deadline(evaluate(pan(PATH, PAN_MS, 30000), true), PAN_MS + 45000, "pan"));
    const main = Math.round((await task()) - before);
    const seen = await stats(r.t0, r.t1);
    await quiet();
    const prefetched = JSON.parse(await evaluate("JSON.stringify(window.__bluemapRsPrefetch ?? null)"));
    row.phases.pan = { ...r, prefetched, tileLatency: latencyStats(latencies), mainMs: main, frames: frames(seen), total: requestStats([...requests.values()]),
      holes: JSON.parse(await evaluate(holes)), from: t };
    await evaluate(teleport(0, 0));
    await sleep(300);
    await waitForTiles(send, seconds);
    await quiet();
    // the page address follows the camera about once a second; the reload should find the start view
    await sleep(1500);
  }
  await load("reload", () => send("Page.reload"), traceReload);
  await send("Page.navigate", { url: "about:blank" });
  await sleep(300);
  await load("revisit", () => send("Page.navigate", { url }));
  row.errors = errors.length;
  row.errorSample = errors.slice(0, 3);
  row.valid = true;
} catch (e) {
  row.valid = false;
  row.why = String(e?.message ?? e);
} finally {
  await closeBrowser(child, profile);
}
const line = JSON.stringify(row);
if (out) appendFileSync(out, `${line}\n`);
const p = row.phases;
console.log(`${label} ${row.net} run ${runIndex}: ${row.valid ? "ok" : `VOID ${row.why}`} first ${p.first?.ms} onscreen ${p.first?.onscreenMs} reload ${p.reload?.ms} revisit ${p.revisit?.ms} pan p50 ${p.pan?.tileLatency?.p50}`);
// the browser's helper processes can keep handles open past its kill
process.exit(0);
