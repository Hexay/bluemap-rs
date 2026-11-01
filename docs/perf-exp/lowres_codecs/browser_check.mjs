// usage: node browser_check.mjs <emit dir> <chrome>
// Uploads each emitted file to a WebGL2 texture the two ways the webapp does (<img>; fetch + createImageBitmap with
// colorSpaceConversion:"none"), with three.js's pixel-store settings, reads it back and compares with the source RGBA.
import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { mkdtempSync, readFileSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const [dir, chrome] = process.argv.slice(2);
const TYPES = { png: "image/png", webp: "image/webp", rgba: "application/octet-stream", dims: "text/plain" };

const PAGE = `<!doctype html><pre id=o>running</pre><script>
const gl = new OffscreenCanvas(4, 4).getContext("webgl2");
function readBack(image, w, h) {
  const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
  gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
  const f = gl.createFramebuffer(); gl.bindFramebuffer(gl.FRAMEBUFFER, f);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, t, 0);
  const out = new Uint8Array(w * h * 4); gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, out);
  gl.deleteFramebuffer(f); gl.deleteTexture(t);
  return out;
}
function diff(a, b) {
  const d = { zero: 0, partial: 0, opaque: 0, maxDelta: 0 };
  for (let i = 0; i < a.length; i += 4) {
    let delta = 0;
    for (let c = 0; c < 4; c++) delta = Math.max(delta, Math.abs(a[i + c] - b[i + c]));
    if (!delta) continue;
    d.maxDelta = Math.max(d.maxDelta, delta);
    a[i + 3] === 0 ? d.zero++ : a[i + 3] === 255 ? d.opaque++ : d.partial++;
  }
  return d;
}
(async () => {
  const r = [];
  for (const [id, files] of Object.entries(await (await fetch("list")).json())) {
    const [w, h] = (await (await fetch(id + ".dims")).text()).split(" ").map(Number);
    const src = new Uint8Array(await (await fetch(id + ".rgba")).arrayBuffer());
    for (const file of files) {
      const img = new Image(); img.src = file; await img.decode();
      const viaImg = diff(src, readBack(img, w, h));
      const blob = await (await fetch(file)).blob();
      const t0 = performance.now();
      const bitmap = await createImageBitmap(blob, { colorSpaceConversion: "none" });
      const ms = performance.now() - t0;
      const viaBitmap = diff(src, readBack(bitmap, w, h));
      r.push({ file, bytes: blob.size, decodeMs: +ms.toFixed(1), viaImg, viaBitmap });
    }
  }
  await fetch("result", { method: "POST", body: JSON.stringify(r) });
})().catch((e) => fetch("result", { method: "POST", body: JSON.stringify({ error: String(e) }) }));
</script>`;

const list = {};
for (const f of readdirSync(dir)) {
  const m = f.match(/^(\w+)\.(.+\.(?:png|webp))$/);
  if (m) (list[m[1]] ??= []).push(f);
}
let browser;
function report(body) {
  const r = JSON.parse(body);
  browser.kill();
  if (r.error) { console.log(r.error); process.exit(1); }
  for (const x of r) console.log(x.file.padEnd(20), String(x.bytes).padStart(8), `${x.decodeMs} ms`, "img", JSON.stringify(x.viaImg), "bitmap", JSON.stringify(x.viaBitmap));
  process.exit(0);
}
const server = createServer((req, res) => {
  const name = decodeURIComponent(req.url.slice(1));
  if (name === "result") {
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => report(body));
    return;
  }
  if (name === "") return res.setHeader("content-type", "text/html").end(PAGE);
  if (name === "list") return res.end(JSON.stringify(list));
  try {
    res.setHeader("content-type", TYPES[name.split(".").pop()]).end(readFileSync(join(dir, name)));
  } catch {
    res.statusCode = 404;
    res.end();
  }
});
server.listen(0, "127.0.0.1", () => {
  const url = `http://127.0.0.1:${server.address().port}/`;
  browser = spawn(chrome, ["--headless=new", "--no-sandbox", "--use-angle=swiftshader", "--enable-unsafe-swiftshader",
    `--user-data-dir=${mkdtempSync(join(tmpdir(), "lowres-chrome-"))}`, url], { stdio: "ignore" });
  setTimeout(() => { console.log("timed out"); browser.kill(); process.exit(1); }, 200000);
});
