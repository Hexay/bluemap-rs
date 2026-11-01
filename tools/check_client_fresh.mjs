// usage: node tools/check_client_fresh.mjs <server base url> <map id> <chrome> [--tune '{"ahead":1}'] [--script unpack.js --wasm unpack.wasm]
// The client script against a proxy that plays a render and failures (docs/18), real webapp, headless Chrome, GL draws off.
// A tile "updated" here is served with another tile's bytes and a new version, so the page's vertex count tells which
// one it shows. Checks: a changed tile shows after a reload and unchanged ones skip the network; a failed version list,
// a corrupt body and a failed first request fall back with no hole; a live "tile" event shows the new tile, also when it
// arrives while that tile is loading. Exits 1 on any failure.
import { spawn } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { pathToFileURL } from "node:url";

const args = process.argv.slice(2);
const option = (name) => {
  const at = args.indexOf(name);
  return at >= 0 ? args.splice(at, 2)[1] : null;
};
const tune = option("--tune");
const scriptPath = option("--script");
const wasmPath = option("--wasm");
const [server, mapId, browser] = args;
const devtools = await import(pathToFileURL(join(import.meta.dirname, "../crates/bm-wasm/bench/devtools.mjs")).href);
const replacement = scriptPath ? devtools.assembleScript(scriptPath, wasmPath) : null;
const DEADLINE_MS = 60000;
const within = (p, what, ms = DEADLINE_MS) => Promise.race([p, sleep(ms, null, { ref: false }).then(() => { throw new Error(`${what}: timed out`); })]);

const NEW_VERSION = "abc-abc-abc";
const T = { x: 1, z: 0 }; // near the view's center, so every load wants it
const ALT = { x: -40, z: -40 }; // far from it: its bytes stand in for the render's output
const path = ({ x, z }) => {
  const part = (n) => (n < 0 ? "-" : "") + [...String(Math.abs(n))].join("/") + "/";
  return `/maps/${mapId}/tiles/0/${`x${part(x)}z${part(z)}`.slice(0, -1)}.prbm`;
};

const state = { changed: false, failVersions: false, corruptOnce: false, failing: false, failed: 0, holdT: null, seen: [], sse: new Set() };
const proxy = createServer(async (req, res) => {
  const url = new URL(req.url, "http://x");
  state.seen.push(url.pathname);
  if (url.pathname.endsWith("/live/sse")) {
    res.writeHead(200, { "Content-Type": "text/event-stream", "Cache-Control": "no-store" });
    res.write(":\n\n");
    state.sse.add(res);
    req.on("close", () => state.sse.delete(res));
    return;
  }
  if (url.pathname.includes("/tile-versions/") && state.failVersions) return void res.writeHead(500).end();
  const isT = url.pathname === path(T);
  if (state.failing && req.headers["x-ahead"]) {
    state.failed++;
    return void res.writeHead(500).end();
  }
  // decided before the hold: this reply left the server before the render
  const swapped = isT && state.changed;
  if (isT && state.holdT) await state.holdT;
  // identity: the proxy may rewrite bodies; packed tiles then come as the frame the script decodes itself
  const headers = { ...req.headers, "accept-encoding": "identity" };
  delete headers.host;
  delete headers.connection;
  if (swapped) {
    delete headers["if-none-match"];
    if ((req.headers["if-none-match"] ?? "").includes(NEW_VERSION)) return void res.writeHead(304, { ETag: `"${NEW_VERSION}-bmq3-zstd"`, "Cache-Control": "public, max-age=86400" }).end();
  }
  const upstream = await fetch(new URL(swapped ? path(ALT) : req.url, server), { headers });
  const out = {};
  upstream.headers.forEach((v, k) => { if (!["transfer-encoding", "connection", "content-length"].includes(k)) out[k] = v; });
  let body = Buffer.from(await upstream.arrayBuffer());
  // fetch decoded it: send it as not encoded
  delete out["content-encoding"];
  if (swapped) out.etag = `"${NEW_VERSION}-bmq3-zstd"`;
  if (isT && state.corruptOnce) {
    state.corruptOnce = false;
    body = Buffer.from("corrupt packed tile");
  }
  if (url.pathname.includes("/tile-versions/") && state.changed) {
    const line = new RegExp(`^0 ${T.x} ${T.z} .*$`, "m");
    body = Buffer.from(body.toString().replace(line, `0 ${T.x} ${T.z} ${NEW_VERSION}`));
  }
  res.writeHead(upstream.status, out).end(upstream.status === 204 || upstream.status === 304 ? undefined : body);
});
await new Promise((done) => proxy.listen(0, "127.0.0.1", done));
const URL_ = `http://127.0.0.1:${proxy.address().port}/#${mapId}:0:70:0:60:0:0.9:0:0:perspective`;

const profile = mkdtempSync(join(tmpdir(), "bm-fresh-"));
const child = spawn(browser, ["--headless=new", "--remote-debugging-port=0", `--user-data-dir=${profile}`, "--enable-unsafe-swiftshader",
  "--window-size=1280,800", "--no-first-run", "--disable-gl-drawing-for-tests", "about:blank"], { stdio: "ignore" });
const failures = [];
const check = (ok, what) => {
  console.log(`${ok ? "ok  " : "FAIL"} ${what}`);
  if (!ok) failures.push(what);
};
try {
  let onNavigated = () => {};
  const send = await devtools.connect(profile, (method, params) => {
    if (method === "Page.frameNavigated" && !params.frame.parentId) onNavigated();
    else if (method === "Fetch.requestPaused") devtools.fulfillScript(send, params.requestId, replacement);
  });
  await send("Page.enable");
  await send("Runtime.enable");
  if (replacement !== null) await send("Fetch.enable", { patterns: [{ urlPattern: devtools.CLIENT_SCRIPT_PATTERN }] });
  if (tune) await send("Page.addScriptToEvaluateOnNewDocument", { source: `localStorage.setItem("bluemap-rs-client-tuning", ${JSON.stringify(tune)})` });
  // marks the script's fetch-aheads (the only low-priority fetches) for the proxy; runs before the script captures fetch
  await send("Page.addScriptToEvaluateOnNewDocument", { source: `{ const f = window.fetch; window.fetch = function (i, o) {
    return f.call(this, i, o?.priority === "low" ? { ...o, headers: { ...o.headers, "X-Ahead": "1" } } : o); }; }` });
  const value = async (expression) => (await send("Runtime.evaluate", { expression })).result.value;
  const count = () => value(`(() => { const t = bluemap.mapViewer.map.hiresTileManager.tiles.get("x${T.x}z${T.z}");
    return t && t.model ? t.model.geometry.attributes.position.count : (t ? -1 : -2); })()`);
  const holes = () => value(`[...bluemap.mapViewer.map.hiresTileManager.tiles.values()].filter((t) => !t.model).length`);
  const load = async (navigate) => {
    const navigated = new Promise((done) => (onNavigated = done));
    await navigate();
    await within(navigated, "navigation");
    const view = await within(devtools.waitForTiles(send, 40), "tiles");
    return view;
  };
  const reload = () => load(() => send("Page.reload"));
  const hiresRequests = () => state.seen.filter((p) => /\/tiles\/0\//.test(p)).length;

  const first = await load(() => send("Page.navigate", { url: URL_ }));
  check(first.loaded === first.wanted && first.wanted > 0, `first load: ${first.loaded}/${first.wanted} tiles`);
  const before = await count();
  await sleep(1500);

  state.seen = [];
  const same = await reload();
  check(same.loaded === same.wanted, `reload, nothing changed: ${same.loaded}/${same.wanted} tiles, ${hiresRequests()} hires requests`);
  check(hiresRequests() === 0, "reload, nothing changed: no hires tile went over the network");
  check(state.seen.some((p) => p.includes("/tile-versions/")), "reload, nothing changed: version list used");
  check((await count()) === before, "reload, nothing changed: same tile");

  state.changed = true;
  state.seen = [];
  const changed = await reload();
  const altCount = await count();
  check(altCount !== before && altCount > 0, `reload after a render: changed tile shown (vertices ${before} -> ${altCount})`);
  check(changed.loaded === changed.wanted && (await holes()) === 0, `reload after a render: ${changed.loaded}/${changed.wanted}, no holes`);
  check(state.seen.filter((p) => p === path(T)).length >= 1 && hiresRequests() <= 3, `reload after a render: ${hiresRequests()} hires requests, the changed one among them`);
  state.changed = false;

  state.failVersions = true;
  state.seen = [];
  const failed = await reload();
  check(failed.loaded === failed.wanted && (await holes()) === 0 && hiresRequests() >= failed.wanted, `version list failing: normal revalidation, ${hiresRequests()} hires requests, no holes`);
  state.failVersions = false;

  state.corruptOnce = true;
  const corrupt = await load(async () => {
    await send("Page.navigate", { url: "about:blank" });
    await sleep(200);
    await send("Network.enable");
    await send("Network.setCacheDisabled", { cacheDisabled: true });
    await send("Page.navigate", { url: URL_ });
  });
  check(corrupt.loaded === corrupt.wanted && (await holes()) === 0, `corrupt body: refetched, no holes (${corrupt.loaded}/${corrupt.wanted})`);
  await send("Network.setCacheDisabled", { cacheDisabled: false });

  // a failed fetch-ahead must leave the tile to the webapp's own request
  await send("Network.setCacheDisabled", { cacheDisabled: true });
  state.failing = true;
  await value(`(() => { const c = bluemap.mapViewer.controlsManager; const t0 = performance.now();
    const step = () => { const u = Math.min(1, (performance.now() - t0) / 3000); c.position.x = 200 * u; if (u < 1) setTimeout(step, 16); }; step(); })()`);
  await sleep(1000);
  state.failing = false;
  await sleep(4000);
  const moved = await within(devtools.waitForTiles(send, 30), "tiles after a failing prefetch");
  check(state.failed > 0 && moved.loaded === moved.wanted && (await holes()) === 0, `${state.failed} fetch-aheads failing during a pan: ${moved.loaded}/${moved.wanted}, no holes`);
  await value(`(() => { const c = bluemap.mapViewer.controlsManager; c.position.x = 0; c.position.z = 0; })()`);
  await sleep(1000);
  await within(devtools.waitForTiles(send, 30), "tiles back home");
  await send("Network.setCacheDisabled", { cacheDisabled: false });

  // live events: the webapp listens once it has a map
  await sleep(1000);
  const connected = await within((async () => { while (!state.sse.size) await sleep(100); return true; })(), "SSE connect", 20000);
  const tileEvent = () => state.sse.forEach((res) => res.write(`event: tile\ndata: ${JSON.stringify({ lod: 0, x: T.x, y: T.z })}\n\n`));
  state.changed = true;
  tileEvent();
  const shown = async (want) => {
    for (let i = 0; i < 100; i++) {
      if ((await count()) === want) return true;
      await sleep(100);
    }
    return false;
  };
  check(connected && (await shown(altCount)), "live tile event: new tile shown");
  state.changed = false;
  tileEvent();
  check(await shown(before), "live tile event: back to the first version");

  // the event comes while the tile is being fetched: the webapp ignores it, so the script has to catch it
  let release;
  state.holdT = new Promise((done) => (release = done));
  await send("Network.setCacheDisabled", { cacheDisabled: true });
  await value(`(() => { const m = bluemap.mapViewer.map.hiresTileManager; const t = m.tiles.get("x${T.x}z${T.z}"); t.load(m.tileLoader, true); })()`);
  await sleep(300);
  state.changed = true;
  tileEvent();
  await sleep(100);
  const held = state.holdT;
  state.holdT = null;
  release();
  await held;
  check(await shown(altCount), "live tile event during that tile's load: the new tile ends up shown");
  await send("Network.setCacheDisabled", { cacheDisabled: false });
  check((await holes()) === 0, "no holes at the end");
} catch (e) {
  check(false, `error: ${e.stack ?? e}`);
} finally {
  await devtools.closeBrowser(child, profile);
}
console.log(failures.length ? `${failures.length} FAILED` : "all ok");
process.exit(failures.length ? 1 : 0);
