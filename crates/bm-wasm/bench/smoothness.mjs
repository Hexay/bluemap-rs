// usage: node crates/bm-wasm/bench/smoothness.mjs <chrome exe> --out rows.jsonl [--rows j529,rc,ro,rn] [--nets 20,50+50,10]
//   [--runs 5] [--uncapped] [--scale 1] [--host devbox] [--port 18100]      resumes: runs already valid in --out are skipped
// node crates/bm-wasm/bench/smoothness.mjs --report rows.jsonl
// Frame pacing of the real webapp in a headed Chrome on the real GPU, fresh profile per run, against servers on a remote
// host through `ssh -L` (docs/perf-exp/smoothness_server.sh, copied there as ~/smooth/server.sh; the tunnel holds
// ~/bench.lock). Results: docs/results/webapp-smoothness-2026-11-01.md. The window must stay visible: occlusion tracking
// is left on, so a covered window turns document.hidden and voids the run.
import { spawn, spawnSync } from "node:child_process";
import { existsSync, appendFileSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { CLIENT_SCRIPT_PATTERN, closeBrowser, connect, fulfillScript, waitForTiles } from "./devtools.mjs";
import { chromeMemory, onBattery, outsideLoad, processes, waitQuiet } from "./smoothness_host.mjs";
import { NOW, RECORDER, RENDERER, pan, slice, teleport, zoom } from "./smoothness_page.mjs";
import { frameStats, latencyStats, report } from "./smoothness_stats.mjs";

const args = process.argv.slice(2);
const option = (name, fallback = null) => {
  const at = args.indexOf(name);
  return at >= 0 ? args.splice(at, 2)[1] : fallback;
};
const flag = (name) => {
  const at = args.indexOf(name);
  return at >= 0 ? !!args.splice(at, 1) : false;
};
const reportFile = option("--report");
if (reportFile) {
  report(reportFile);
  process.exit(0);
}
const out = resolve(option("--out") ?? "");
const rows = option("--rows", "j529,rc,ro,rn").split(",");
const nets = option("--nets", "20,50+50,10").split("+");
const runs = Number(option("--runs", "5"));
const uncapped = flag("--uncapped");
const scale = option("--scale");
const host = option("--host", "devbox");
const port = Number(option("--port", "18100"));
const browser = args[0];
if (!browser || !args.length) throw new Error("usage: smoothness.mjs <chrome exe> --out rows.jsonl");

const URL_ = `http://127.0.0.1:${port}/#real:0:70:0:60:0:0.9:0:0:perspective`;
const PATH = [[0, 0], [360, 0], [360, 360]];
const PAN_MS = 12000;
const ZOOM_MS = 8000;
const ZOOM_FAR = 1500;
const SETTLE_MS = 45000;
const TILES_S = 120;
const MAX_OUTSIDE = 0.15;
const COLLAPSED_MS = 25;

const remote = (command) => {
  const r = spawnSync("ssh", ["-o", "ConnectTimeout=10", host, command], { encoding: "utf8" });
  if (r.status !== 0) throw new Error(`ssh ${command}: ${r.stderr}${r.stdout}`);
  return r.stdout.trim();
};

// Holds the lock and the forward for as long as this process lives; ssh dying closes stdin there and frees the lock.
async function openTunnel() {
  const ssh = spawn("ssh", ["-o", "ExitOnForwardFailure=yes", "-L", `${port}:127.0.0.1:8100`, host,
    `exec flock ~/bench.lock sh -c 'echo locked; cat > /dev/null'`], { stdio: ["pipe", "pipe", "inherit"] });
  await new Promise((done, fail) => {
    ssh.stdout.once("data", done);
    ssh.once("exit", (code) => fail(new Error(`tunnel exited: ${code}`)));
  });
  ssh.removeAllListeners("exit");
  return ssh;
}

async function tunnelRtt() {
  const ms = [];
  for (let i = 0; i < 7; i++) {
    const t = performance.now();
    await (await fetch(`http://127.0.0.1:${port}/settings.json`)).arrayBuffer();
    ms.push(performance.now() - t);
  }
  return Number(ms.toSorted((a, b) => a - b)[3].toFixed(1));
}

async function run(row, net, runIndex, tunnelPid) {
  const quiet = await waitQuiet(0.25, 1000, 600);
  if (!quiet) return { valid: false, why: "machine stayed busy" };
  const serverLoad = remote(`bash ~/smooth/server.sh start ${row}`);
  const tunnelRttMs = await tunnelRtt();
  const profile = mkdtempSync(join(tmpdir(), "bm-smooth-"));
  const [latency, mbit] = net.split(",").map(Number);
  const child = spawn(browser, ["--remote-debugging-port=0", `--user-data-dir=${profile}`, "--window-size=1280,800",
    "--no-first-run", "--no-default-browser-check", "--disable-extensions",
    ...(uncapped ? ["--disable-gpu-vsync", "--disable-frame-rate-limit"] : []),
    ...(scale ? [`--force-device-scale-factor=${scale}`] : []), "about:blank"], { stdio: "ignore" });
  try {
    let onNavigated = () => {};
    const workers = new Set();
    const net_ = { requests: 0, bytes: 0 };
    // http only: textures.json turns into ~2000 data: URL loads
    const inflight = new Set();
    let exceptions = 0;
    const send = await connect(profile, (method, params) => {
      if (method === "Page.frameNavigated" && !params.frame.parentId) onNavigated();
      else if (method === "Fetch.requestPaused") fulfillScript(send, params.requestId, "");
      else if (method === "Target.attachedToTarget" && params.targetInfo.type === "worker") workers.add(params.sessionId);
      else if (method === "Target.detachedFromTarget") workers.delete(params.sessionId);
      else if (method === "Runtime.exceptionThrown") exceptions++;
      else if (method === "Network.requestWillBeSent") {
        if (/^https?:/.test(params.request.url) && !params.request.url.includes("/live/") && params.type !== "EventSource") inflight.add(params.requestId);
      } else if (method === "Network.loadingFailed") inflight.delete(params.requestId);
      else if (method === "Network.loadingFinished" && inflight.delete(params.requestId)) {
        net_.requests++;
        net_.bytes += params.encodedDataLength;
      }
    });
    const evaluate = async (expression, awaitPromise = false) => {
      const r = await send("Runtime.evaluate", { expression, awaitPromise });
      if (r.exceptionDetails) throw new Error(`page: ${r.exceptionDetails.exception?.description ?? r.exceptionDetails.text}`);
      return r.result.value;
    };
    const metric = async (name) => (await send("Performance.getMetrics")).metrics.find((m) => m.name === name).value;
    await send("Page.enable");
    await send("Runtime.enable");
    await send("Performance.enable");
    await send("Page.bringToFront");
    await send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true });
    await send("Page.addScriptToEvaluateOnNewDocument", { source: RECORDER });
    if (row === "rn") await send("Fetch.enable", { patterns: [{ urlPattern: CLIENT_SCRIPT_PATTERN }] });
    await send("Network.enable");
    await send("Network.emulateNetworkConditions", { offline: false, latency,
      downloadThroughput: (mbit * 1e6) / 8, uploadThroughput: (mbit * 1e6) / 8 });

    const before = processes();
    const started = Date.now();
    const phases = {};
    let hidden = 0;
    let lostFocus = 0;
    // t0/t1 on the page's clock; a navigation restarts it and the main-thread counter
    const measure = async (name, fn) => {
      const busy0 = name === "reload" ? 0 : await metric("TaskDuration");
      const net0 = { ...net_ };
      const { t0, t1, ...extra } = await fn();
      const mainMs = Math.round(((await metric("TaskDuration")) - busy0) * 1e3);
      const netDuring = { requests: net_.requests - net0.requests, kb: Math.round((net_.bytes - net0.bytes) / 1024) };
      const seen = JSON.parse(await evaluate(slice(t0, t1)));
      hidden += seen.hiddenAt.length + (seen.hidden ? 1 : 0);
      lostFocus += seen.focus ? 0 : 1;
      let workerHeapMb = null;
      if (workers.size) {
        workerHeapMb = 0;
        for (const session of workers) {
          const h = await send("Runtime.getHeapUsage", {}, session).catch(() => null);
          if (h) workerHeapMb += (h.usedSize + (h.backingStorageSize ?? 0)) / 1048576;
        }
      }
      const memory = { jsHeapMb: Math.round((await metric("JSHeapUsedSize")) / 1048576), workerHeapMb,
        chrome: chromeMemory(processes(), profile) };
      phases[name] = { ...frameStats(seen.intervals, seen.tasks), ms: Math.round(t1 - t0), mainMs, net: netDuring, memory, ...extra };
    };
    const load = (navigate) => async () => {
      const navigated = new Promise((done) => (onNavigated = done));
      await navigate();
      await navigated;
      const view = await waitForTiles(send, TILES_S);
      if (!view || view.loaded < view.wanted) throw new Error(`tiles never loaded: ${JSON.stringify(view)}`);
      return { t0: 0, t1: view.ms, wanted: view.wanted };
    };
    const idle = async () => {
      const t0 = await evaluate(NOW);
      await sleep(5000);
      return { t0, t1: await evaluate(NOW) };
    };
    // gotcha: a hidden window stops rAF, so motion never ends; the deadline voids the run instead of hanging it
    const driven = (expression, ms) => Promise.race([evaluate(expression, true),
      sleep(ms, null, { ref: false }).then(() => { throw new Error("motion missed its deadline: window hidden?"); })]);
    const panPhase = async () => {
      const { latencies, ...r } = JSON.parse(await driven(pan(PATH, PAN_MS, SETTLE_MS), PAN_MS + SETTLE_MS + 15000));
      return { ...r, tileLatency: latencyStats(latencies) };
    };
    // back to the start of the path outside any timed phase
    const home = async () => {
      await evaluate(teleport(0, 0));
      await sleep(300);
      await waitForTiles(send, TILES_S);
      await sleep(500);
    };

    await measure("load", load(() => send("Page.navigate", { url: URL_ })));
    const gpu = JSON.parse(await evaluate(RENDERER));
    if (/swiftshader|llvmpipe|basic render|software/i.test(gpu.renderer)) return { valid: false, why: `software renderer: ${gpu.renderer}`, gpu };
    await measure("idle", idle);
    // lowres outlasts the hires tiles; a backlog at 10 Mbit would be charged to the pan
    for (let q = 0; q < 5;) {
      q = inflight.size ? 0 : q + 1;
      await sleep(100);
    }
    const allInMs = Math.round(await evaluate(NOW));
    await measure("pan", panPhase);
    await home();
    await measure("panAgain", panPhase);
    await home();
    await measure("zoom", async () => {
      return JSON.parse(await driven(zoom(ZOOM_FAR, ZOOM_MS), ZOOM_MS + 15000));
    });
    // the page address follows the camera about once a second; the reload should find the start view
    await sleep(1500);
    await measure("reload", load(() => send("Page.reload")));
    const gpuEnd = JSON.parse(await evaluate(RENDERER));
    const after = processes();
    const seconds = (Date.now() - started) / 1000;
    const isBench = (pid, p) => pid === process.pid || pid === tunnelPid || p.cmd.includes(profile) || /Get-CimInstance/.test(p.cmd);
    const outside = outsideLoad(before, after, seconds, isBench);
    const tunnel = (after.get(tunnelPid)?.cpuS ?? 0) - (before.get(tunnelPid)?.cpuS ?? 0);
    const why = /swiftshader|llvmpipe|basic render|software/i.test(gpuEnd.renderer) ? `fell back to ${gpuEnd.renderer}`
      : hidden ? "window hidden during the run"
      : phases.idle.p50 > COLLAPSED_MS ? `frame rate collapsed: idle p50 ${phases.idle.p50} ms`
      : outside.share > MAX_OUTSIDE ? `outside load ${(outside.share * 100).toFixed(0)}%: ${outside.top.join(" ")}`
      : null;
    return { valid: !why, why, gpu, phases, allInMs, seconds: Math.round(seconds), quiet, tunnelRttMs, tunnelCpuPct: (tunnel * 100) / seconds,
      outside, lostFocus, exceptions, serverLoad: Number(serverLoad.split("\n").at(-1)), battery: onBattery() };
  } finally {
    await closeBrowser(child, profile);
  }
}

const done = new Set();
if (existsSync(out)) {
  for (const line of readFileSync(out, "utf8").trim().split("\n").filter(Boolean)) {
    const r = JSON.parse(line);
    if (r.valid) done.add(`${r.row}|${r.net}|${r.run}`);
  }
}
const tunnel = await openTunnel();
console.log(`locked and tunneled; ${remote("bash ~/smooth/server.sh prepare")}`);
try {
  const backwards = (xs, n) => (n % 2 ? xs : xs.toReversed());
  for (let r = 1; r <= runs; r++) {
    for (const net of backwards(nets, r)) {
      for (const row of backwards(rows, r)) {
        for (let attempt = 1; attempt <= 3 && !done.has(`${row}|${net}|${r}`); attempt++) {
          let result;
          try {
            result = await run(row, net, r, tunnel.pid);
          } catch (e) {
            result = { valid: false, why: `error: ${e.message}` };
          }
          appendFileSync(out, JSON.stringify({ row, net, run: r, attempt, uncapped, at: new Date().toISOString(), ...result }) + "\n");
          const p = result.phases;
          console.log(`${row} ${net} run ${r}: ${result.valid ? "ok" : `VOID ${result.why}`}` + (p ? ` load ${p.load.ms} pan p99 ${p.pan.p99} idle p50 ${p.idle.p50}` : ""));
          if (result.valid) done.add(`${row}|${net}|${r}`);
        }
      }
    }
  }
} finally {
  try {
    remote("bash ~/smooth/server.sh stop");
  } catch (e) {
    console.log(e.message);
  }
  tunnel.kill();
}
process.exit(0);
