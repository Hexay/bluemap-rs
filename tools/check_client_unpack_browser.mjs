// usage: node tools/check_client_unpack_browser.mjs <map url> <chrome or edge exe> [screenshot.png] [seconds=25]
// Client-side unpacking (docs/18) in a real browser: opens the webapp headless over the DevTools protocol, records
// how its own hires tile requests were answered, then refetches those tiles in the page both ways (through the
// client script, and as server-made PRBM over XMLHttpRequest, which the script does not touch) and compares the
// bytes. Fails if no tile arrived packed, a tile differs, or the page logged an error.
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const [url, browser, screenshot, seconds = "25"] = process.argv.slice(2);
if (!url || !browser) {
  console.error("usage: node tools/check_client_unpack_browser.mjs <map url> <browser exe> [screenshot.png] [seconds]");
  process.exit(2);
}
const MEDIA_TYPE = "application/vnd.bluemap.bmq3";
const PORT = 9333;
const profile = mkdtempSync(join(tmpdir(), "bm-unpack-"));
// swiftshader: headless machines have no GPU for the webapp's WebGL
const flags = ["--headless=new", `--remote-debugging-port=${PORT}`, `--user-data-dir=${profile}`,
  "--enable-unsafe-swiftshader", "--window-size=1280,800", "--no-first-run", "about:blank"];
const child = spawn(browser, flags, { stdio: "ignore" });

async function pageSocket() {
  for (let attempt = 0; attempt < 100; attempt++) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      const page = targets.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      // not listening yet
    }
    await sleep(200);
  }
  throw new Error("the browser's DevTools port never opened");
}

const tiles = new Map(); // requestId → { url, type, encoding, wire }
const problems = [];
let exitCode = 1;
try {
  const socket = new WebSocket(await pageSocket());
  await new Promise((resolve, reject) => {
    socket.onopen = resolve;
    socket.onerror = reject;
  });
  let nextId = 0;
  const pending = new Map();
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      pending.set(++nextId, { resolve, reject });
      socket.send(JSON.stringify({ id: nextId, method, params }));
    });
  socket.onmessage = ({ data }) => {
    const message = JSON.parse(data);
    if (message.id) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      return message.error ? reject(new Error(message.error.message)) : resolve(message.result);
    }
    const p = message.params;
    if (message.method === "Network.responseReceived" && /\/tiles\/0\/.*\.prbm$/.test(p.response.url)) {
      const header = (name) => Object.entries(p.response.headers).find(([k]) => k.toLowerCase() === name)?.[1];
      tiles.set(p.requestId, { url: p.response.url, status: p.response.status, type: header("content-type"),
        encoding: header("content-encoding") ?? "identity", wire: 0 });
    } else if (message.method === "Network.loadingFinished" && tiles.has(p.requestId)) {
      tiles.get(p.requestId).wire = p.encodedDataLength;
    } else if (message.method === "Runtime.exceptionThrown") {
      problems.push(`exception: ${p.exceptionDetails.exception?.description ?? p.exceptionDetails.text}`);
    } else if (message.method === "Runtime.consoleAPICalled" && (p.type === "error" || p.type === "warning")) {
      problems.push(`console.${p.type}: ${p.args.map((a) => a.value ?? a.description ?? "").join(" ")}`);
    }
  };

  await send("Network.enable");
  await send("Runtime.enable");
  await send("Page.enable");
  await send("Page.navigate", { url });
  await sleep(Number(seconds) * 1000);

  const loaded = [...tiles.values()].filter((t) => t.status === 200);
  const packed = loaded.filter((t) => (t.type ?? "").startsWith(MEDIA_TYPE));
  const compare = async (urls) => {
    const viaXhr = (u) =>
      new Promise((resolve, reject) => {
        const xhr = new XMLHttpRequest();
        xhr.open("GET", u);
        xhr.responseType = "arraybuffer";
        xhr.onload = () => resolve(new Uint8Array(xhr.response));
        xhr.onerror = reject;
        xhr.send();
      });
    const result = { same: 0, different: [], prbmBytes: 0, quads: 0 };
    for (const u of urls) {
      const unpacked = new Uint8Array(await (await fetch(u)).arrayBuffer());
      const fromServer = await viaXhr(u);
      const same = unpacked.length === fromServer.length && unpacked.every((byte, i) => byte === fromServer[i]);
      if (same) result.same += 1;
      else result.different.push(`${u}: ${unpacked.length} B unpacked here, ${fromServer.length} B from the server`);
      result.prbmBytes += fromServer.length;
      result.quads += (fromServer[2] | (fromServer[3] << 8) | (fromServer[4] << 16)) / 6;
    }
    return result;
  };
  const evaluated = await send("Runtime.evaluate", {
    expression: `(${compare})(${JSON.stringify(packed.slice(0, 40).map((t) => t.url))})`,
    awaitPromise: true,
    returnByValue: true,
  });
  if (evaluated.exceptionDetails) throw new Error(evaluated.exceptionDetails.exception?.description ?? "compare failed");
  const compared = evaluated.result.value;
  if (screenshot) {
    const shot = await send("Page.captureScreenshot", { format: "png" });
    writeFileSync(screenshot, Buffer.from(shot.data, "base64"));
  }

  const wire = (list) => Math.round(list.reduce((sum, t) => sum + t.wire, 0) / Math.max(list.length, 1));
  const codings = [...new Set(packed.map((t) => t.encoding))].join(", ");
  console.log(`webapp loaded ${loaded.length} hires tiles: ${packed.length} packed (${codings}; ${wire(packed)} B on the wire each)`);
  console.log(`refetched ${compared.same + compared.different.length}: ${compared.same} identical to the server's PRBM ` +
    `(${Math.round(compared.prbmBytes / Math.max(compared.same, 1))} B, ${Math.round(compared.quads)} quads in all)`);
  compared.different.forEach((line) => console.log(`DIFFERENT ${line}`));
  problems.slice(0, 20).forEach((line) => console.log(`PAGE ${line}`));
  const ok = packed.length > 0 && compared.same > 0 && compared.different.length === 0 && problems.length === 0;
  exitCode = ok ? 0 : 1;
  socket.close();
} catch (error) {
  console.error(error);
} finally {
  child.kill();
  await sleep(1000);
  try {
    rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 300 });
  } catch {
    // the browser's helper processes can hold the profile a little longer; it is a temp dir
  }
}
process.exit(exitCode);
