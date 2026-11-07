// usage: node tools/webperf/run.mjs <map url> [--chrome PATH] [--scenario initial-load|pan,zoom,flyover,idle,soak]
//   [--runs 1] [--extent 256] [--seconds S] [--minutes 3] [--cpu-profile] [--gl] [--trace] [--net <latency ms>,<Mbit/s>]
//   [--headful] [--window 960,600] [--chrome-args "…"] [--inject patch.js,…] [--out results.jsonl] [--label name]
//   [--cpus 2] [--max-rss 2500] [--run-timeout 180] [--max-minutes 15] [--wait-load <psi> [--wait-max 10]]
//   [--contended-psi 40]
// Frame pacing, main-thread cost per frame, draw calls, memory and tile latency of the real webapp under scripted
// camera scenarios (docs/25). One Chrome at a time, fresh profile per (scenario, run), each in a capped systemd scope
// (box.mjs: --cpus, --max-rss); hard limits per run and per invocation; every result records the box's load and is
// labelled contended or not. Headless renders with SwiftShader: frame intervals there are CPU-rendered, the
// GPU-independent numbers are main ms/frame, draw calls, memory and load times.
import { appendFileSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { connect } from "../../crates/bm-wasm/bench/devtools.mjs";
import { boxRecord, boxState, installTeardown, launch, preflight, QUIET_CHROME_FLAGS, reapStaleScopes, stopAll,
  waitForCalm, waitForTurn } from "./box.mjs";
import { summarizeProfile } from "./cpuprofile.mjs";
import { medianTable } from "./rows.mjs";
import { startTrace, traceEvent } from "./trace.mjs";
import { driveExpression, SCENARIOS } from "./scenarios.mjs";

const args = process.argv.slice(2);
const flag = (name) => (args.includes(name) ? (args.splice(args.indexOf(name), 1), true) : false);
const option = (name, fallback = null) => {
  const at = args.indexOf(name);
  return at >= 0 ? args.splice(at, 2)[1] : fallback;
};
const chrome = option("--chrome", join(homedir(), ".cache/ms-playwright/chromium-1248/chrome-linux64/chrome"));
const scenarios = option("--scenario", "initial-load").split(",");
const runs = Number(option("--runs", "1"));
const extent = Number(option("--extent", "256"));
const seconds = Number(option("--seconds", NaN)) || undefined;
const minutes = Number(option("--minutes", "3"));
const cpus = Number(option("--cpus", "2"));
const maxRss = Number(option("--max-rss", "2500"));
const runTimeout = Number(option("--run-timeout", "180"));
const maxMinutes = Number(option("--max-minutes", "15"));
const waitLoad = option("--wait-load");
const waitMax = Number(option("--wait-max", "10"));
const contendedPsi = Number(option("--contended-psi", "40"));
// scripts added at document start after the probe, e.g. a candidate fix from tools/webperf/patches/
const inject = (option("--inject")?.split(",") ?? []).map((f) => readFileSync(f, "utf8")).join("\n");
const net = option("--net")?.split(",").map(Number);
const out = option("--out");
const label = option("--label", "");
const cpuProfile = flag("--cpu-profile");
const headful = flag("--headful");
const trace = flag("--trace");
const windowSize = option("--window", "960,600");
// extra browser switches, space separated: e.g. "--use-gl=angle --use-angle=gl" for Mesa under xvfb-run --headful
const chromeArgs = option("--chrome-args", "").split(" ").filter(Boolean);
const startProbe = `bmPerf.start({ gl: ${flag("--gl")} })`;
const [url] = args;
if (headful && !process.env.DISPLAY) throw new Error("--headful needs a display (e.g. under xvfb-run)");
if (!url) throw new Error("usage: run.mjs <map url> [options]; see the header");
const probe = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "probe.js"), "utf8");
const started = Date.now();
const log = (m) => process.stderr.write(`[${((Date.now() - started) / 1000).toFixed(0)}s] ${m}\n`);
if (trace) log("warning: --trace is expensive (several times the CPU and memory of a plain run); use it for one run");
const evaluate = async (send, expression) => {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
  return r.result.value;
};
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

async function measure(scenario, send, errors, traceEvents) {
  let stopTrace = null;
  const beginCapture = async () => {
    if (cpuProfile) await send("Profiler.start");
    if (trace) stopTrace = await startTrace(send, traceEvents);
  };
  await send("Page.enable");
  await send("Runtime.enable");
  await send("Performance.enable", { timeDomain: "threadTicks" });
  const initial = scenario === "initial-load";
  await send("Page.addScriptToEvaluateOnNewDocument", { source: `${probe}\n${inject}\n${initial ? `${startProbe};` : ""}` });
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
  return { ...result, ...report,
    cdp: { taskMsPerS: Math.round(((after.TaskDuration - before.TaskDuration) * 1e3) / secs),
      scriptMsPerS: Math.round(((after.ScriptDuration - before.ScriptDuration) * 1e3) / secs),
      layoutStyleMsPerS: Math.round(((after.LayoutDuration + after.RecalcStyleDuration
        - before.LayoutDuration - before.RecalcStyleDuration) * 1e3) / secs),
      jsHeapMB: Math.round(after.JSHeapUsedSize / 2 ** 20), nodes: after.Nodes },
    cpu: prof, trace: traced, errors: errors.length };
}

// one browser, one scenario; rejects with { limit } when --run-timeout passes (the browser's scope is stopped first)
async function run(scenario, boxBefore) {
  const profile = mkdtempSync(join(tmpdir(), "bm-webperf-"));
  // Ubuntu 23.10+ blocks the unprivileged user namespaces Chrome's sandbox needs; the page is our own local server
  const sandbox = process.platform === "linux" ? ["--no-sandbox"] : [];
  const browser = launch(chrome, [...(headful ? [] : ["--headless=new"]), ...sandbox, "--remote-debugging-port=0",
    `--user-data-dir=${profile}`, "--enable-unsafe-swiftshader", "--enable-precise-memory-info",
    `--window-size=${windowSize}`, "--no-first-run", "--no-default-browser-check", ...QUIET_CHROME_FLAGS, ...chromeArgs,
    "about:blank"], { cpus, maxRssMB: maxRss, maxSeconds: runTimeout + 30, tag: profile, log });
  log(`${scenario}: Chrome ${browser.child.pid}${browser.unit ? ` in ${browser.unit}.scope` : ""}`);
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(Object.assign(new Error(`run timeout: --run-timeout ${runTimeout} s passed`),
      { limit: "--run-timeout" })), runTimeout * 1000);
  });
  let stats = null;
  try {
    const errors = [];
    const traceEvents = [];
    const measured = (async () => {
      const send = await connect(profile, (method, params) => {
        if (method === "Runtime.exceptionThrown") errors.push(params.exceptionDetails.exception?.description);
        else traceEvent(traceEvents, method, params);
      });
      return measure(scenario, send, errors, traceEvents);
    })();
    const result = await Promise.race([measured, timeout]);
    stats = browser.stats();
    return { ...result, box: boxRecord(boxBefore, boxState(), stats, contendedPsi) };
  } catch (e) {
    stats ??= browser.stats();
    e.box = boxRecord(boxBefore, boxState(), stats, contendedPsi);
    if (stats.oomKills) e.message += ` (Chrome hit MemoryMax --max-rss ${maxRss} MB: ${stats.oomKills} OOM kill(s))`;
    throw e;
  } finally {
    clearTimeout(timer);
    const left = browser.stop();
    if (left) log(`killed ${left} Chrome process(es) left after stopping the browser`);
    rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}

const all = [];
const record = (scenario, i, r) => {
  if (out) appendFileSync(out, JSON.stringify({ label, url, scenario, run: i, at: new Date().toISOString(), ...r }) + "\n");
};
const summary = () => {
  if (!all.length) return;
  console.log(`medians of ${all.length} run(s)${label ? `, ${label}` : ""} (software GL: frame times are CPU-rendered)`);
  console.table(medianTable(all, (r) => `${r.scenario}${r.box.contended ? " (contended)" : ""}`));
};
// prints what was measured, then exits non-zero
const abort = (message, code) => {
  log(message);
  stopAll();
  summary();
  process.exit(code);
};

installTeardown(log);
reapStaleScopes(log);
for (const s of scenarios) {
  if (s === "initial-load") continue;
  if (!SCENARIOS[s]) abort(`unknown scenario ${s}`, 1);
  const drive = SCENARIOS[s]({ extent, seconds, minutes }).seconds;
  if (drive + 60 > runTimeout) abort(`${s} drives ${drive} s: raise --run-timeout above ${drive + 60}`, 1);
}
setTimeout(() => abort(`invocation limit: --max-minutes ${maxMinutes} passed; Chrome stopped`, 3), maxMinutes * 60e3);
for (const scenario of scenarios) {
  for (let i = 0; i < runs; i++) {
    if (!(await waitForTurn(waitMax * 60e3, log))) abort(`another webperf run stayed live for --wait-max ${waitMax} min`, 4);
    if (waitLoad !== null && !(await waitForCalm(Number(waitLoad), waitMax * 60e3, log)))
      log(`warning: cpu PSI stayed above --wait-load ${waitLoad} for --wait-max ${waitMax} min: running anyway`);
    let before;
    try {
      before = preflight(maxRss);
    } catch (e) {
      abort(`${scenario} #${i}: ${e.message}`, 4);
    }
    log(`${scenario} #${i}: load1 ${before.load1}, cpu PSI some avg10 ${before.cpuPsiSome10}, `
      + `MemAvailable ${before.memAvailableMB} MB`);
    let r;
    try {
      r = await run(scenario, before);
    } catch (e) {
      record(scenario, i, { aborted: e.message, box: e.box });
      abort(`${scenario} #${i}: ${e.message}`, e.limit ? 2 : 1);
    }
    all.push({ scenario, ...r });
    record(scenario, i, r);
    const b = r.box;
    log(`${scenario} #${i}: Chrome cpu ${b.cpuMs} ms, throttled ${b.throttledMs} ms, mem peak ${b.memPeakMB} MB`
      + `${b.contended ? `, contended (cpu PSI > ${contendedPsi})` : ""}`);
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
}
summary();
process.exit(0);
