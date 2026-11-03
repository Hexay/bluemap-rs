// usage: node tools/webperf/run.mjs <map url> [--chrome PATH] [--scenario initial-load,pan,zoom,flyover,idle,soak]
//   [--runs 3] [--extent 256] [--seconds S] [--minutes 5] [--cpu-profile] [--net <latency ms>,<Mbit/s>] [--headful]
//   [--out results.jsonl] [--label name]
// Frame pacing, main-thread cost per frame, draw calls, memory and tile latency of the real webapp under scripted
// camera scenarios (docs/25). Fresh Chrome profile per (scenario, run). Headless renders with SwiftShader: frame
// intervals there are CPU-rendered, the GPU-independent numbers are main ms/frame, draw calls, memory and load times.
import { spawn } from "node:child_process";
import { appendFileSync, mkdtempSync, readFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { closeBrowser, connect } from "../../crates/bm-wasm/bench/devtools.mjs";
import { summarizeProfile } from "./cpuprofile.mjs";
import { startTrace, traceEvent } from "./trace.mjs";
import { driveExpression, SCENARIOS } from "./scenarios.mjs";

const args = process.argv.slice(2);
const flag = (name) => (args.includes(name) ? (args.splice(args.indexOf(name), 1), true) : false);
const option = (name, fallback = null) => {
  const at = args.indexOf(name);
  return at >= 0 ? args.splice(at, 2)[1] : fallback;
};
const chrome = option("--chrome", join(homedir(), ".cache/ms-playwright/chromium-1248/chrome-linux64/chrome"));
const scenarios = option("--scenario", "initial-load,pan,zoom,flyover,idle").split(",");
const runs = Number(option("--runs", "3"));
const extent = Number(option("--extent", "256"));
const seconds = Number(option("--seconds", NaN)) || undefined;
const minutes = Number(option("--minutes", "5"));
const net = option("--net")?.split(",").map(Number);
const out = option("--out");
const label = option("--label", "");
const cpuProfile = flag("--cpu-profile");
const headful = flag("--headful");
const trace = flag("--trace");
const windowSize = option("--window", "1280,800");
const startProbe = `bmPerf.start({ gl: ${flag("--gl")} })`;
const [url] = args;
if (!url) throw new Error("usage: run.mjs <map url> [options]; see the header");
const probe = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "probe.js"), "utf8");
const evaluate = async (send, expression) => {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
  return r.result.value;
};
const started = Date.now();
const log = (m) => process.stderr.write(`[${((Date.now() - started) / 1000).toFixed(0)}s] ${m}\n`);
// after a navigation: retried while the new document replaces the context
async function waitForView(send) {
  for (let i = 0; i < 100; i++) {
    try {
      const v = await evaluate(send, "bmPerf.untilComplete(60)");
      if (v) return v;
    } catch {
      await sleep(100);
    }
  }
  throw new Error("the webapp never reported its tiles");
}
const metrics = async (send) =>
  Object.fromEntries((await send("Performance.getMetrics")).metrics.map((m) => [m.name, m.value]));

async function run(scenario) {
  const profile = mkdtempSync(join(tmpdir(), "bm-webperf-"));
  // Ubuntu 23.10+ blocks the unprivileged user namespaces Chrome's sandbox needs; the page is our own local server
  const sandbox = process.platform === "linux" ? ["--no-sandbox"] : [];
  const child = spawn(chrome, [...(headful ? [] : ["--headless=new"]), ...sandbox, "--remote-debugging-port=0",
    `--user-data-dir=${profile}`, "--enable-unsafe-swiftshader", "--enable-precise-memory-info",
    `--window-size=${windowSize}`, "--no-first-run", "--no-default-browser-check", "about:blank"], { stdio: "ignore" });
  try {
    const errors = [];
    const traceEvents = [];
    const send = await connect(profile, (method, params) => {
      if (method === "Runtime.exceptionThrown") errors.push(params.exceptionDetails.exception?.description);
      else traceEvent(traceEvents, method, params);
    });
    let stopTrace = null;
    const beginCapture = async () => {
      if (cpuProfile) await send("Profiler.start");
      if (trace) stopTrace = await startTrace(send, traceEvents);
    };
    await send("Page.enable");
    await send("Runtime.enable");
    await send("Performance.enable", { timeDomain: "threadTicks" });
    const initial = scenario === "initial-load";
    await send("Page.addScriptToEvaluateOnNewDocument", { source: probe + (initial ? `\n${startProbe};` : "") });
    if (net) {
      await send("Network.enable");
      await send("Network.emulateNetworkConditions", { offline: false, latency: net[0],
        downloadThroughput: (net[1] * 1e6) / 8, uploadThroughput: (net[1] * 1e6) / 8 });
    }
    if (cpuProfile) {
      await send("Profiler.enable");
      await send("Profiler.setSamplingInterval", { interval: 250 });
    }
    const m0 = initial ? await metrics(send) : null;
    if (initial) await beginCapture();
    await send("Page.navigate", { url });
    const view = await waitForView(send);
    log(`${scenario}: view ${view.ms} ms, complete ${view.complete}, hires ${view.hires.loaded}/${view.hires.wanted}, `
      + `lowres ${view.lowres.map((l) => `${l.loaded}/${l.wanted}`).join(" ")}`);
    let result = { view };
    let before = m0;
    if (!initial) {
      const sc = SCENARIOS[scenario]({ extent, seconds, minutes });
      before = await metrics(send);
      await evaluate(send, startProbe);
      await beginCapture();
      await evaluate(send, driveExpression(sc.path, sc.seconds, extent));
      // how long the view takes to fill in once the camera stops
      const t0 = Date.now();
      const settle = await evaluate(send, "bmPerf.untilComplete(performance.now() / 1000 + 30)");
      result.settle = { ms: Date.now() - t0, complete: settle.complete };
      log(`${scenario}: driven ${sc.seconds} s, settled in ${result.settle.ms} ms`);
    } else {
      await evaluate(send, "new Promise((r) => setTimeout(r, 3000))"); // the frames right after the view completes
    }
    const prof = cpuProfile ? summarizeProfile((await send("Profiler.stop")).profile) : null;
    const traced = stopTrace ? await stopTrace() : null;
    await evaluate(send, "bmPerf.stop()");
    const after = await metrics(send);
    const report = await evaluate(send, "bmPerf.report()");
    const secs = report.seconds;
    result = { ...result, ...report,
      cdp: { taskMsPerS: Math.round(((after.TaskDuration - before.TaskDuration) * 1e3) / secs),
        scriptMsPerS: Math.round(((after.ScriptDuration - before.ScriptDuration) * 1e3) / secs),
        layoutStyleMsPerS: Math.round(((after.LayoutDuration + after.RecalcStyleDuration
          - before.LayoutDuration - before.RecalcStyleDuration) * 1e3) / secs),
        jsHeapMB: Math.round(after.JSHeapUsedSize / 2 ** 20), nodes: after.Nodes },
      cpu: prof, trace: traced, errors: errors.length };
    return result;
  } finally {
    await closeBrowser(child, profile);
  }
}

const median = (xs) => {
  const v = xs.filter((x) => typeof x === "number" && !Number.isNaN(x)).sort((a, b) => a - b);
  return v.length ? v[Math.floor(v.length / 2)] : null;
};
const ROWS = {
  "view ms": (r) => r.view.ms,
  "settle ms": (r) => r.settle?.ms,
  "terrainHeight ms/frame p95": (r) => r.terrainHeightMs.p95,
  "rAF fps": (r) => r.fps,
  "rendered fps": (r) => r.renderedFps,
  "frame p95 ms": (r) => r.frameInterval.p95,
  "frames >50ms": (r) => r.frameInterval.over50,
  "main ms/frame p50": (r) => r.renderLoopMs.p50,
  "main ms/frame p95": (r) => r.renderLoopMs.p95,
  "main busy ms/s": (r) => r.cdp.taskMsPerS,
  "draw calls p50": (r) => r.drawCalls.p50,
  "triangles k p50": (r) => r.trianglesK.p50,
  "long tasks": (r) => r.longTasks.n,
  "longest task ms": (r) => r.longTasks.max,
  "hires tile ms p50": (r) => r.tileMs.hires.p50,
  "hires tile ms p95": (r) => r.tileMs.hires.p95,
  "lowres tile ms p95": (r) => r.tileMs.lowres.p95,
  "tiles fetched": (r) => r.net.hires.n + r.net.lowres.n,
  "kB fetched": (r) => r.net.hires.kB + r.net.lowres.kB,
  "heap MB peak": (r) => r.memory.peakHeapMB,
  "geometries end": (r) => r.memory.end?.geometries,
  "textures end": (r) => r.memory.end?.textures,
  "hires geom MB peak": (r) => r.memory.peakHiresGeomMB,
};

const table = {};
for (const scenario of scenarios) {
  if (scenario !== "initial-load" && !SCENARIOS[scenario]) throw new Error(`unknown scenario ${scenario}`);
  const results = [];
  for (let i = 0; i < runs; i++) {
    const r = await run(scenario);
    results.push(r);
    if (out) appendFileSync(out, JSON.stringify({ label, url, scenario, run: i, at: new Date().toISOString(), ...r }) + "\n");
    if (r.cpu) {
      console.log(`${scenario} #${i} cpu busy ${r.cpu.busyMs} ms: ${JSON.stringify(r.cpu.buckets)}`);
      r.cpu.top.slice(0, 8).forEach((f) => console.log(`  ${String(f.pct).padStart(5)}%  ${f.fn}`));
    }
    if (r.trace) {
      console.log(`${scenario} #${i} thread busy ms: ${JSON.stringify(r.trace.threadsBusyMs)}`);
      console.log(`  main self ms: ${r.trace.mainTop.slice(0, 10).map((e) => `${e.name} ${e.ms}`).join(", ")}`);
    }
    if (r.gl) console.log(`${scenario} #${i} GL ms (calls): ${r.gl.slice(0, 6).map((g) => `${g.fn} ${g.ms} (${g.n})`).join(", ")}`);
  }
  table[scenario] = Object.fromEntries(Object.entries(ROWS).map(([k, f]) => [k, median(results.map((r) => {
    try {
      return f(r);
    } catch {
      return null;
    }
  }))]));
}
console.log(`medians of ${runs} run(s)${label ? `, ${label}` : ""}${headful ? "" : " (headless: SwiftShader, frame times CPU-rendered)"}`);
console.table(table);
process.exit(0);
