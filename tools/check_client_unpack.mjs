// usage: node tools/check_client_unpack.mjs <server url> <compat webroot> [tiles=300]
// End-to-end check of client-side unpacking (docs/18) without a browser: runs the script the server's index.html
// loads (WASM included) in Node, fetches hires tiles of an optimized storage through it, and compares each with
// the PRBM in a compat webroot of the same map. Fails if a tile differs or none arrived packed.
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { gunzipSync } from "node:zlib";

const [server, compat, limit = "300"] = process.argv.slice(2);
if (!server || !compat) {
  console.error("usage: node tools/check_client_unpack.mjs <server url> <compat webroot> [tiles=300]");
  process.exit(2);
}
const base = server.endsWith("/") ? server : server + "/";

const index = await (await fetch(base)).text();
const script = /<script src="\.\/(assets\/bluemap-rs-unpack-[^"]+)"/.exec(index)?.[1];
if (!script) {
  console.error("index.html loads no client script: is the map's storage optimized and client-unpack on?");
  process.exit(1);
}

let packed = 0;
let packedBytes = 0;
const plain = globalThis.fetch;
globalThis.fetch = async (...args) => {
  const response = await plain(...args);
  if ((response.headers.get("Content-Type") || "").startsWith("application/vnd.bluemap.bmq3")) {
    packed += 1;
    packedBytes += Number(response.headers.get("Content-Length") || 0);
  }
  return response;
};
globalThis.location = { href: base };
(0, eval)(await (await plain(base + script)).text());

const tiles = [];
const walk = (dir) => {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) walk(path);
    else if (entry.name.endsWith(".prbm.gz")) tiles.push(path);
  }
};
for (const map of readdirSync(join(compat, "maps"))) walk(join(compat, "maps", map, "tiles", "0"));
tiles.sort();
const step = Math.max(1, Math.floor(tiles.length / Number(limit)));

let checked = 0;
let prbmBytes = 0;
for (let i = 0; i < tiles.length; i += step) {
  const url = base + relative(compat, tiles[i]).replaceAll("\\", "/").replace(/\.gz$/, "");
  const got = Buffer.from(await (await fetch(url)).arrayBuffer());
  const want = gunzipSync(readFileSync(tiles[i]));
  if (!got.equals(want)) {
    console.error(`${url}: ${got.length} bytes through the client script, ${want.length} in the compat webroot`);
    process.exit(1);
  }
  checked += 1;
  prbmBytes += want.length;
}
console.log(
  `${checked} of ${tiles.length} hires tiles identical; ${packed} arrived packed ` +
    `(${Math.round(packedBytes / Math.max(packed, 1))} B on the wire each, ${Math.round(prbmBytes / checked)} B of PRBM)`,
);
process.exit(packed > 0 ? 0 : 1);
