// The DevTools side of the real-webapp harnesses (view.mjs, gpu_trace.mjs).
import { readFileSync, readdirSync, rmSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

// The page target's socket URL of a Chrome started with --remote-debugging-port=0 --user-data-dir=<profile>:
// the port it picked is in <profile>/DevToolsActivePort, so parallel runs never share one.
export async function pageSocket(profile) {
  for (let attempt = 0; attempt < 150; attempt++) {
    try {
      const port = readFileSync(join(profile, "DevToolsActivePort"), "utf8").split("\n")[0].trim();
      const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      const page = targets.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      // not up yet
    }
    await sleep(200);
  }
  throw new Error("the browser's DevTools port never opened");
}

// Returns send(method, params); every protocol event goes to onEvent(method, params).
export async function connect(profile, onEvent) {
  const socket = new WebSocket(await pageSocket(profile));
  await new Promise((resolve, reject) => ((socket.onopen = resolve), (socket.onerror = reject)));
  let nextId = 0;
  const pending = new Map();
  socket.onmessage = ({ data }) => {
    const m = JSON.parse(data);
    if (!m.id) return onEvent(m.method, m.params);
    const { resolve, reject } = pending.get(m.id);
    pending.delete(m.id);
    return m.error ? reject(new Error(m.error.message)) : resolve(m.result);
  };
  // a crashed or killed browser: fail what is waiting instead of letting Node exit with it unsettled
  socket.onclose = () => {
    for (const { reject } of pending.values()) reject(new Error("the browser closed the DevTools connection"));
    pending.clear();
  };
  return (method, params = {}, sessionId = undefined) =>
    new Promise((resolve, reject) => {
      pending.set(++nextId, { resolve, reject });
      socket.send(JSON.stringify({ id: nextId, method, params, sessionId }));
    });
}

// The client script as bm-web assembles it; a file from before it had parts works too.
export function assembleScript(scriptPath, wasmPath) {
  const parts = () => readdirSync(dirname(scriptPath)).filter((f) => f.endsWith(".js") && f !== basename(scriptPath)).sort()
    .map((f) => readFileSync(join(dirname(scriptPath), f), "utf8")).join("");
  return readFileSync(scriptPath, "utf8").replace("// __CLIENT_PARTS__", parts)
    .replace("__UNPACK_WASM_BASE64__", readFileSync(wasmPath).toString("base64"));
}

// Fetch.enable pattern of the server's client script; fulfillScript answers its Fetch.requestPaused.
export const CLIENT_SCRIPT_PATTERN = "*bluemap-rs-unpack-*";
export const fulfillScript = (send, requestId, body) =>
  send("Fetch.fulfillRequest", { requestId, responseCode: 200,
    responseHeaders: [{ name: "Content-Type", value: "text/javascript" }],
    body: Buffer.from(body).toString("base64") });

// in the page: resolves once the hires tiles of the view are in
const tilesLoadedExpression = (seconds) => `new Promise((resolve) => {
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

// After a navigation has committed: the view's numbers, null if the page never answered.
export async function waitForTiles(send, seconds) {
  for (let i = 0; i < 100; i++) {
    try {
      const r = await send("Runtime.evaluate", { expression: tilesLoadedExpression(seconds), awaitPromise: true });
      if (r.result?.value) return JSON.parse(r.result.value);
    } catch {
      await sleep(100); // context replaced while navigating
    }
  }
  return null;
}

export async function closeBrowser(child, profile) {
  child.kill();
  await sleep(800);
  try {
    rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 300 });
  } catch {
    // helper processes may hold the profile a moment longer; it is a temp dir
  }
}
