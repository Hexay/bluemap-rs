// usage: node crates/bm-wasm/bench/load_breakdown.mjs <view.mjs --dump file> [--fetch <server base url>]
// Per category of a load: requests, wire and decoded bytes, encodings, cache headers; what the 204s are. --fetch
// re-reads the first load's files from the server and prints what other encodings of them would weigh.
import { deflateSync, gzipSync, inflateSync, brotliCompressSync, zstdCompressSync, constants as z } from "node:zlib";
import { readFileSync } from "node:fs";

const args = process.argv.slice(2);
const fetchAt = args.indexOf("--fetch");
const base = fetchAt >= 0 ? args.splice(fetchAt, 2)[1] : null;
const rows = readFileSync(args[0], "utf8").trim().split("\n").map((l) => JSON.parse(l));

const lodOf = (path) => path.match(/\/tiles\/(\d+)\//)?.[1];
function category(url) {
  const path = new URL(url).pathname;
  if (path.includes("/live/")) return "live data";
  const lod = lodOf(path);
  if (lod === "0") return "hires tiles";
  if (lod) return `lowres LOD ${lod}`;
  if (path.endsWith("/textures.json") || path.endsWith("/textures.json.gz")) return "textures.json";
  if (/\/maps\/[^/]+\/settings\.json$/.test(path)) return "map settings.json";
  if (path.includes("bluemap-rs-unpack")) return "client script (JS + WASM)";
  if (/(^\/|\.html)$/.test(path)) return "HTML";
  if (/\.js$/.test(path)) return "webapp JS";
  if (/\.css$/.test(path)) return "webapp CSS";
  if (/\.(ttf|woff2?)$/.test(path)) return "font";
  if (/\.conf$/.test(path)) return "lang";
  if (path === "/settings.json") return "root settings.json";
  return "other";
}
const header = (r, name) => r.responseHeaders?.[Object.keys(r.responseHeaders).find((k) => k.toLowerCase() === name)];
const distinct = (rs, f) => [...new Set(rs.map(f).filter(Boolean))].join("; ") || "-";
const kb = (n) => (n / 1024).toFixed(1);
// a cache hit's encodedDataLength is 0 anyway; a failed request has none
const wire = (r) => (r.servedFromCache || r.fromDiskCache ? 0 : r.bytes ?? 0);
const requestBytes = (r) => Object.entries(r.requestHeaders ?? {}).reduce((n, [k, v]) => n + k.length + v.length + 4, 0);

function table(phase) {
  const rs = rows.filter((r) => r.phase === phase);
  const groups = Map.groupBy(rs, (r) => category(r.url));
  console.log(`\n### ${phase}: ${rs.length} requests, ${kb(rs.reduce((n, r) => n + wire(r), 0))} KB wire\n`);
  console.log("| category | n | statuses | wire KB | decoded KB | req hdr KB | encoding | cache-control | etag | last-modified | caller |");
  console.log("|---|---:|---|---:|---:|---:|---|---|---|---|---|");
  for (const [name, g] of [...groups].sort(([a], [b]) => a.localeCompare(b))) {
    const counts = {};
    for (const r of g) {
      const s = r.servedFromCache || r.fromDiskCache ? "cache" : r.failed ? "failed" : r.status ?? "?";
      counts[s] = (counts[s] ?? 0) + 1;
    }
    console.log(`| ${name} | ${g.length} | ${Object.entries(counts).map(([k, v]) => `${v}×${k}`).join(" ")} `
      + `| ${kb(g.reduce((n, r) => n + wire(r), 0))} | ${kb(g.reduce((n, r) => n + r.decoded, 0))} `
      + `| ${kb(g.reduce((n, r) => n + requestBytes(r), 0))} | ${distinct(g, (r) => header(r, "content-encoding"))} `
      + `| ${distinct(g, (r) => header(r, "cache-control"))} | ${g.filter((r) => header(r, "etag")).length}/${g.length} `
      + `| ${g.filter((r) => header(r, "last-modified")).length}/${g.length} | ${distinct(g.slice(0, 3), (r) => r.caller)} |`);
  }
  const conditional = rs.filter((r) => r.requestHeaders?.["If-None-Match"] || r.requestHeaders?.["if-none-match"]).length;
  console.log(`\nrequests with If-None-Match: ${conditional}; 304s: ${rs.filter((r) => r.status === 304).length}`);
}

const coord = (s) => Number(s.replaceAll("/", ""));
function tileRanges(phase, keep) {
  const out = new Map();
  for (const r of rows.filter((x) => x.phase === phase && keep(x))) {
    const m = new URL(r.url).pathname.match(/\/tiles\/(\d+)\/x(-?[\d/]+)\/?z(-?[\d/]+)/);
    if (!m) continue;
    const e = out.get(m[1]) ?? { n: 0, x: [Infinity, -Infinity], z: [Infinity, -Infinity] };
    const [x, zz] = [coord(m[2]), coord(m[3])];
    e.n++;
    e.x = [Math.min(e.x[0], x), Math.max(e.x[1], x)];
    e.z = [Math.min(e.z[0], zz), Math.max(e.z[1], zz)];
    out.set(m[1], e);
  }
  return out;
}

function empties() {
  const first = rows.filter((r) => r.phase === "first");
  const absent = first.filter((r) => r.status === 204);
  console.log(`\n### 204s (first load): ${absent.length}, ${absent.reduce((n, r) => n + wire(r), 0)} B wire down, `
    + `${absent.reduce((n, r) => n + requestBytes(r), 0)} B request headers up`);
  console.log(`span ${absent.length ? Math.round((Math.max(...absent.map((r) => r.end)) - Math.min(...absent.map((r) => r.start))) * 1e3) : 0} ms; `
    + `median ${absent.length ? absent.map((r) => (r.end - r.start) * 1e3).sort((a, b) => a - b)[absent.length >> 1].toFixed(1) : 0} ms; `
    + `sample: ${absent.slice(0, 4).map((r) => new URL(r.url).pathname).join(" ")}`);
  console.log(`initiators: ${distinct(absent, (r) => `${r.type}/${r.initiator}`)}; headers: ${JSON.stringify(absent[0]?.responseHeaders)}`);
  const ok = tileRanges("first", (r) => r.status === 200);
  for (const [lod, e] of [...tileRanges("first", (r) => r.status === 204)].sort())
    console.log(`LOD ${lod}: 204 n=${e.n} x ${e.x} z ${e.z}; 200 n=${ok.get(lod)?.n} x ${ok.get(lod)?.x} z ${ok.get(lod)?.z}`);
  const reloaded = rows.filter((r) => r.phase === "reload" && r.failed);
  console.log(`reload failures: ${reloaded.length}, errors ${distinct(reloaded, (r) => `${r.failed}/canceled=${r.canceled}`)}, `
    + `types ${distinct(reloaded, (r) => r.type)}, lods ${distinct(reloaded, (r) => lodOf(new URL(r.url).pathname))}`);
}

const sizes = (body) => ({
  gz6: gzipSync(body, { level: 6 }).length, gz9: gzipSync(body, { level: 9 }).length,
  br11: brotliCompressSync(body, { params: { [z.BROTLI_PARAM_QUALITY]: 11 } }).length,
  zstd19: zstdCompressSync(body, { params: { [z.ZSTD_c_compressionLevel]: 19 } }).length,
});

// [IHDR fields, inflated scanlines, stored size of everything else]
function pngParts(png) {
  const idat = [];
  let [width, height, depth, colour, other] = [0, 0, 0, 0, 8];
  for (let at = 8; at < png.length;) {
    const len = png.readUInt32BE(at);
    const type = png.toString("latin1", at + 4, at + 8);
    if (type === "IHDR") [width, height, depth, colour] = [png.readUInt32BE(at + 8), png.readUInt32BE(at + 12), png[at + 16], png[at + 17]];
    if (type === "IDAT") idat.push(png.subarray(at + 8, at + 8 + len));
    other += type === "IDAT" ? 12 : len + 12;
    at += len + 12;
  }
  return { width, height, depth, colour, raw: inflateSync(Buffer.concat(idat)), other };
}

const paeth = (a, b, c) => {
  const p = a + b - c, pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
};

// Re-filter 8-bit RGBA scanlines (they must already be filter None) with one fixed filter.
function refiltered({ width, height, raw }, filter) {
  const stride = width * 4 + 1;
  const out = Buffer.alloc(raw.length);
  for (let y = 0; y < height; y++) {
    out[y * stride] = filter;
    for (let i = 1; i < stride; i++) {
      const at = y * stride + i;
      const a = i > 4 ? raw[at - 4] : 0;
      const b = y > 0 ? raw[at - stride] : 0;
      const c = i > 4 && y > 0 ? raw[at - stride - 4] : 0;
      const pred = filter === 1 ? a : filter === 2 ? b : filter === 3 ? (a + b) >> 1 : paeth(a, b, c);
      out[at] = (raw[at] - pred) & 0xff;
    }
  }
  return out;
}

async function reencodings() {
  const urls = [...new Set(rows.filter((r) => r.phase === "first" && r.status === 200).map((r) => r.url))]
    .filter((u) => !["hires tiles", "live data"].includes(category(u)));
  const totals = new Map();
  const add = (cat, values) => {
    const t = totals.get(cat) ?? {};
    for (const [k, v] of Object.entries(values)) t[k] = (t[k] ?? 0) + v;
    totals.set(cat, t);
  };
  for (const u of urls) {
    const path = new URL(u).pathname;
    const res = await fetch(new URL(path, base), { headers: { "Accept-Encoding": "identity" } });
    let body = Buffer.from(await res.arrayBuffer());
    if (path.endsWith(".gz")) body = Buffer.from(await new Response(new Blob([body]).stream().pipeThrough(new DecompressionStream("gzip"))).arrayBuffer());
    const cat = category(u);
    if (cat.startsWith("lowres") || path.endsWith(".png")) {
      if (body.toString("latin1", 1, 4) !== "PNG") continue;
      const p = pngParts(body);
      const none = p.raw.every === undefined ? false : p.colour === 6 && p.depth === 8 && [...Array(p.height).keys()].every((y) => p.raw[y * (p.width * 4 + 1)] === 0);
      const row = { n: 1, stored: body.length, z9: deflateSync(p.raw, { level: 9 }).length + p.other, none: none ? 1 : 0 };
      if (none) {
        row.bestFilter = Math.min(row.z9, ...[1, 2, 3, 4].map((f) => deflateSync(refiltered(p, f), { level: 9 }).length + p.other));
        for (const f of [1, 2, 3, 4]) row[`f${f}`] = deflateSync(refiltered(p, f), { level: 9 }).length + p.other;
      }
      add(cat, row);
    } else {
      add(cat, { n: 1, stored: body.length, ...sizes(body) });
    }
  }
  console.log("\n### the first load's files, re-encoded (bytes)\n");
  for (const [cat, t] of [...totals].sort(([a], [b]) => a.localeCompare(b))) console.log(`${cat}: ${JSON.stringify(t)}`);
}

for (const phase of [...new Set(rows.map((r) => r.phase))]) table(phase);
empties();
if (base) await reencodings();
