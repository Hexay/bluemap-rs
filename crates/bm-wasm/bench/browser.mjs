// usage: node crates/bm-wasm/bench/browser.mjs <corpus dir> <chrome exe> [unpack.wasm=crates/bm-web/client/unpack.wasm]
//   [reps=5] [--cpu-prof out.cpuprofile] [--pipeline '{"workers": 2, "handover": "stream"}'] [--client unpack.js]
// Headless Chrome over the DevTools protocol, the corpus and bench scripts served over HTTP (cross-origin isolated,
// for 5 µs timers); prints one JSON line. Default: WASM unpack time per tile (core.mjs). --pipeline: the whole
// client path (pipeline.mjs), first pass cold then `reps` warm, main-thread time from Performance.getMetrics.
// --client: the same for a client script as the server ships it (client.mjs), the wasm argument inlined into it.
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as sleep } from "node:timers/promises";
import { pageSocket } from "./devtools.mjs";

const args = process.argv.slice(2);
const option = (name) => {
  const at = args.indexOf(name);
  return at >= 0 ? args.splice(at, 2)[1] : null;
};
const profOut = option("--cpu-prof");
const clientScript = option("--client");
const pipeline = option("--pipeline") ?? (clientScript ? "{}" : null);
const [dir, browser, wasm = "crates/bm-web/client/unpack.wasm", reps = "5"] = args;
const here = dirname(fileURLToPath(import.meta.url));
const files = {
  "/": ["text/html", "<!doctype html><title>bench</title>"],
  "/core.mjs": ["text/javascript", readFileSync(join(here, "core.mjs"))],
  "/pipeline.mjs": ["text/javascript", readFileSync(join(here, "pipeline.mjs"))],
  "/client.mjs": ["text/javascript", readFileSync(join(here, "client.mjs"))],
  "/unpack.wasm": ["application/wasm", readFileSync(wasm)],
};
if (clientScript) {
  const script = readFileSync(clientScript, "utf8").replace("__UNPACK_WASM_BASE64__", readFileSync(wasm).toString("base64"));
  files["/client-script.js"] = ["text/javascript", script];
}
const isolation = { "Cross-Origin-Opener-Policy": "same-origin", "Cross-Origin-Embedder-Policy": "require-corp" };
const server = createServer((req, res) => {
  const known = files[req.url];
  if (known) return res.writeHead(200, { "Content-Type": known[0], ...isolation }).end(known[1]);
  let body;
  try {
    body = readFileSync(join(dir, req.url.slice(1)));
  } catch {
    return res.writeHead(404).end();
  }
  res.writeHead(200, isolation).end(body);
}).listen(0, "127.0.0.1");
await new Promise((resolve) => server.once("listening", resolve));
const origin = `http://127.0.0.1:${server.address().port}`;

const profile = mkdtempSync(join(tmpdir(), "bm-wasm-bench-"));
const child = spawn(browser, ["--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`,
  "--no-first-run", "about:blank"], { stdio: "ignore" });
let exitCode = 1;
try {
  const socket = new WebSocket(await pageSocket(profile));
  await new Promise((resolve, reject) => ((socket.onopen = resolve), (socket.onerror = reject)));
  let nextId = 0;
  const pending = new Map();
  let loaded;
  const load = new Promise((resolve) => (loaded = resolve));
  socket.onmessage = ({ data }) => {
    const m = JSON.parse(data);
    if (m.method === "Page.loadEventFired") loaded();
    if (!m.id) return;
    const { resolve, reject } = pending.get(m.id);
    pending.delete(m.id);
    m.error ? reject(new Error(m.error.message)) : resolve(m.result);
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      pending.set(++nextId, { resolve, reject });
      socket.send(JSON.stringify({ id: nextId, method, params }));
    });
  const evaluate = async (expression) => {
    const { result, exceptionDetails } = await send("Runtime.evaluate", { expression, awaitPromise: true });
    if (exceptionDetails) throw new Error(exceptionDetails.exception?.description ?? exceptionDetails.text);
    return JSON.parse(result.value);
  };
  const taskMs = async () =>
    (await send("Performance.getMetrics")).metrics.find((m) => m.name === "TaskDuration").value * 1e3;
  await send("Page.enable");
  await send("Performance.enable");
  await send("Page.navigate", { url: `${origin}/` });
  await load;
  if (profOut) {
    await send("Profiler.enable");
    await send("Profiler.setSamplingInterval", { interval: 100 });
    await send("Profiler.start");
  }
  const loadCorpus = `const manifest = await (await fetch("/manifest.json")).json();
    const bodies = await Promise.all(manifest.map(async (t) =>
      ({ ...t, body: new Uint8Array(await (await fetch("/" + t.name)).arrayBuffer()) })));
    const wasm = await (await fetch("/unpack.wasm")).arrayBuffer();`;
  const chrome = await evaluate(`JSON.stringify(navigator.userAgent.match(/Chrome\\/[\\d.]+/)?.[0])`);
  if (pipeline) {
    const { isolated, tiles } = await evaluate(`(async () => {
      ${loadCorpus}
      globalThis.bench = ${clientScript
        ? `await (await import("${origin}/client.mjs")).prepare(await (await fetch("/client-script.js")).text(), bodies, ${pipeline})`
        : `await (await import("${origin}/pipeline.mjs")).prepare(wasm, bodies, ${pipeline})`};
      return JSON.stringify({ isolated: crossOriginIsolated, tiles: bench.tiles ?? bodies.length });
    })()`);
    const passes = [];
    for (let r = 0; r <= Number(reps); r++) {
      const before = await taskMs();
      const pass = await evaluate(`bench.pass().then(JSON.stringify)`);
      passes.push({ ...pass, mainMsPerTile: +((await taskMs() - before) / tiles).toFixed(3) });
    }
    if (clientScript) await evaluate(`bench.pass(true).then(JSON.stringify)`);
    const warm = passes.slice(1);
    const best = (k) => Math.min(...warm.map((p) => p[k]));
    console.log(JSON.stringify({ wasm, chrome, ...JSON.parse(pipeline), isolated, tiles, cold: passes[0],
      warm: { wallMsPerTile: best("wallMsPerTile"), mainMsPerTile: best("mainMsPerTile"),
        ourMainMsPerTile: best("ourMainMsPerTile") } }));
  } else {
    const result = await evaluate(`(async () => {
      const { run } = await import("${origin}/core.mjs");
      ${loadCorpus}
      return JSON.stringify(await run(wasm, bodies, ${Number(reps)}));
    })()`);
    console.log(JSON.stringify({ wasm, chrome, ...result }));
  }
  if (profOut) writeFileSync(profOut, JSON.stringify((await send("Profiler.stop")).profile));
  exitCode = 0;
} catch (error) {
  console.error(error);
} finally {
  child.kill();
  server.close();
  await sleep(500);
  rmSync(profile, { recursive: true, force: true });
}
process.exit(exitCode);
