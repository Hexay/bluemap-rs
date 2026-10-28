# 18 — Client-side unpacking of optimized hires tiles

An `optimized` storage keeps hires tiles as BMQ3 blobs (docs/17), about 12× smaller than the gzip PRBM the webapp
loads. Serving one to the unmodified webapp means unpacking it and compressing 1–2 MB of PRBM again: ~3 ms of
server CPU per tile nobody asked for recently, against 0.08 ms for a compat tile (numbers below). With client-side
unpacking the server sends the blob's content as stored and the browser rebuilds the PRBM.

It is negotiated per request. A client that does not ask gets PRBM exactly as before.

## Wire

- **Request**: a hires tile URL (`maps/<id>/tiles/0/x…/z….prbm`) with `Accept` naming
  `application/vnd.bluemap.bmq3+zstd` (the script decodes zstd frames) or only `application/vnd.bluemap.bmq3`
  (scripts of 0.3.0).
- **Reply**, if the storage packs hires tiles and the tile is stored in model mode, the tile's *model body*
  (docs/17, `bm_format::compact`) in one of:
  - client accepts zstd: the blob's zstd frame, byte for byte from storage, `Content-Encoding: zstd`,
    `Content-Type: application/vnd.bluemap.bmq3`;
  - else, if it named `+zstd`: the same frame without `Content-Encoding`, `Content-Type:
    application/vnd.bluemap.bmq3+zstd` (see "Browsers without zstd");
  - else (a 0.3.0 script) gzip: the frame decompressed and gzipped (transcode cache, ~8 KB per tile); else
    identity; both `application/vnd.bluemap.bmq3`.
- A tile stored raw (the model could not hold it), a `.gz` URL, a compat storage or a server with the feature off
  answers with PRBM and `application/octet-stream`. The client tells the two apart by `Content-Type`.
- `Vary: Accept, Accept-Encoding` on every hires reply of a packing storage while the feature is on.
- ETags: `"<version>-bmq3-zstd"`, `-bmq3-frame`, `-bmq3-gzip`, `-bmq3`, distinct from every PRBM representation's. A revalidation
  from an unpacking client matches the packed tag or the PRBM tag (it holds whichever form the tile had).

Code: `crates/bm-web/src/client_unpack.rs`, `map_data::serve`, `transcode::packed_body`,
`bm_storage::packed_model_frame`.

## Client

`crates/bm-web/client/unpack.js`, with the WASM unpacker (`crates/bm-wasm`, 85 KB) inlined as base64. The webapp
bundle is not modified: BlueMap 5.28 loads tiles through `fetch`, so the script wraps `fetch`, adds the `Accept`
header to hires tile requests, unpacks in workers, and hands the webapp a `Response` holding the unpacked PRBM
(docs/24). Any failure (WASM
unavailable, corrupt body) falls back to an ordinary request.

- The server adds `<script src="./assets/bluemap-rs-unpack-<hash>.js">` in front of the webapp's module script
  when it serves `index.html`, and serves the script at that URL. Both only while a registered map's storage packs
  hires tiles; the files in the webroot are never touched. An `index.html` that already names the script is left
  alone.
- An `index.html` served by something else (nginx in front of the webroot) does not get the tag, so those viewers
  stay on the PRBM path.
- A viewer opts out with `localStorage.setItem("bluemap-rs-client-unpack", "off")` and a reload.
- A server opts out with the hidden `webserver.conf` key `client-unpack: false`.
- The scripted `index.html` goes out with `Cache-Control: no-cache` (on its 304s too) and an ETag ending in the
  script's hash, checked alone when sent: the file's length and mtime survive an upgrade, its script does not.
  Any `assets/bluemap-rs-unpack-<8 hex>.js` is answered with the current script, for pages cached before (Java's
  template headers let browsers keep `index.html` a day).

The unpacker is `bm_format::compact::BodyUnpacker`, the same code the server runs, built without the `codec`
feature, plus zstd's C decoder (`zstd-sys`) for frames sent unlabelled. WebAssembly float arithmetic is IEEE 754,
so positions and normals come out bit-identical. After changing the decoder or `bm-wasm`, rebuild the checked-in
module: `py -3 tools/build_wasm.py`. It needs clang and llvm-ar with the wasm32 target (the devbox has both; this
Windows machine does not: run it there and copy `crates/bm-web/client/unpack.wasm` back). The script links a
staticlib with rust-lld itself: a cdylib exports, and so keeps, every upstream `#[no_mangle]` function, and
zstd-sys's wasm shim brings ~35 KB of `qsort` nobody calls.

## Browsers without zstd

Chrome 123+ and Firefox 126+ send `zstd` in `Accept-Encoding`; Safari since 26.3, and only on iOS/macOS 26.3+
(it uses the system's network stack, so Safari 26 on older macOS does not). Older Safari, older browsers, and
clients behind a proxy that rewrites `Accept-Encoding` used to get the model body gzipped by the server: 0.24 ms of
CPU per tile, 32k tiles/s instead of ~300k, bounded only by the transcode memory gate. The script now decodes the
frame itself (`bmq3_unpack_frame`), so the server sends what it stored; the browser's own zstd stays in use where
it exists (`client_unpack::Form`). Nothing needs `DecompressionStream`.

Decided on numbers, thresholds set first: the decoder ships to every viewer, so at most +35 KB of script on the
wire (fetched once per release, `immutable`; ~8 tiles' worth) and at most +15% unpack time per tile in Chrome
(the alternative, gzip transcodes under the `server-unpack` slots and cache, keeps the server cost and only moves
it into a queue). Devbox, 261 tiles of the 4096² world (every 64th; 89 KB body, 8.4 KB frame each), unpack ms per
tile warm (`crates/bm-wasm/bench`, `node.mjs`/`browser.mjs --frames`, corpus with `.frame` files):

| module | raw | script on the wire (base64, gzip) | Node | Chrome |
|---|---:|---:|---:|---:|
| bodies only (0.3.0) | 45.8 KB | 25.3 KB | 0.454 | 0.423 |
| + ruzstd 0.9, frames | 89.4 KB | 46.4 KB | 0.897 | 0.697 |
| + zstd C, cdylib, `-O3` | 151.5 KB | 72.1 KB | 0.500 | 0.471 |
| + zstd C, linked from a staticlib, `-Os`, X1 Huffman, short sequences (shipped) | **84.7 KB** | **44.9 KB** | 0.481 | **0.447** |
| same, `-O3` | 89.1 KB | 47.5 KB | 0.478 | 0.449 |

ruzstd adds 63% in Chrome; the shipped build adds 0.036 ms (9%) and 19.6 KB of
script. Time to a rendered view (`view.mjs`, 42 tiles, median of 3, devbox headless SwiftShader):
0.3.0 script 4976 ms / this one 4992 ms with Chrome's `Accept-Encoding`; 5228 / 5218 ms with `gzip, deflate, br`;
no script 5720 ms: the decode is noise next to the webapp's own work.

Server, `docs/perf-exp/testbox_web.sh` on the devbox (2026-10-28, locked, load 8–15 from other users; d12 and
d12-opt webroots), rows for a client without zstd:

| client | 1 conn: CPU ms/req | p50 | 32 conns: req/s | CPU ms/req | KB/tile |
|---|---:|---:|---:|---:|---:|
| 0.3.0 script (gzip transcode) | 0.24 | 159 µs | 32,192 | 0.39 | 8 |
| this script (frame as stored) | **0.02** | 16 µs | **299,140** | **0.03** | **7** |
| this script, browser with zstd | 0.02 | 16 µs | 299,731 | 0.03 | 7 |

`tools/check_client_unpack.mjs` (Node, no zstd: frames) 505 tiles identical, 7,498 B each;
`tools/check_client_unpack_browser.mjs` in Chrome 42 of 42 packed both ways (3,831 B as zstd, 3,812 B as
`bmq3+zstd` with `--accept-encoding "gzip, deflate, br"`), 40 refetched byte-identical. Both:
`docs/perf-exp/devbox_client_frames.sh`.

A tab still running the 0.3.0 script keeps the gzip path under the memory gate until it reloads (`index.html`
revalidates, and any script hash is answered with the current script).

## Clients that don't unpack

`webserver.conf` `server-unpack` (generated with a comment; absent = `limited`, so Java configs load unchanged)
decides what a request without the opt-in (or a `.gz` URL) gets for a packed hires tile:

- `limited`: unpacked by the server, at most max(1, cores / 4) at once (`WebOptions::server_unpack_slots`). A
  request without a free slot waits for one, up to 500 ms (`SERVER_UNPACK_WAIT`), in a queue of min(64 × slots,
  256) (`server_unpack_queue`); past the deadline or with the queue full: `503`, `Retry-After: 1`, `no-store`.
- `never`: `406` with a one-line `text/plain` body. Tiles stored raw are still served, under the `limited` slots.
- `always`: every request unpacked, bounded only by the transcode memory gate (cores at once, waiting).

The queue exists for one viewer without the script: a view load asks for ~50 hires tiles at once (all of them
over HTTP/2) and the webapp does not retry a failed tile until it leaves the view distance. 64 per slot holds a
whole view load per slot with room to spare, and drains in ~100–200 ms per slot at 1.5–3 ms a tile, well inside
the deadline. Each waiter holds a blocking-pool thread, so the queue is capped at 256, half of tokio's default 512:
the opt-in requests and storage reads keep the other half (the before-run below: 600 waiting flood requests left
opt-in p50 at ~1 ms). The deadline bounds what an overloaded server promises: a tile not started within 500 ms is
refused rather than queued behind a flood.

A transcode-cache hit and an `If-None-Match` 304 cost no slot; opt-in requests, raw tiles for them, compat
storages and everything but hires tiles never touch the setting. The slot is taken inside the blocking task, after
the tile is read (`gate::Gate::acquire_bounded`): that read is what an opt-in request costs anyway, and deciding there lets
cache hits, 304s and `never`'s raw tiles through exactly. Slot holders skip the memory gate (the slots bound them
tighter); all other transcodes (items, gzip model bodies) keep it.

One viewer without the script loading a view (the 49 hires tiles around the center + 16 lowres, cold cache) on the
devbox below, `limited`, `docs/perf-exp/testbox_viewer.sh` (2026-10-28, load 1–9):

| slots | connections | 200 | 503 | load wall | slowest request |
|---|---:|---:|---:|---:|---:|
| 4 (default) | 6 (HTTP/1.1 browser) | 65 | 0 | 15 ms | 3 ms |
| 4 (default) | 65 (one burst, as HTTP/2) | 65 | 0 | 16 ms | 14 ms |
| 1 (a 4-core host) | 6 | 65 | 0 | 50 ms | 8 ms |
| 1 | 65 | 65 | 0 | 48 ms | 46 ms |

The 1-slot burst used ~10% of the deadline; a host with 3–5× slower cores still fits.

A flood: devbox (Ryzen 9700X, 16 threads, 4 slots, queue 256), the 4096² world, 2026-10-28, locked runs: 32
opt-in connections on one half of the tiles, alone and with a closed-loop flood of the other half without the
opt-in from the same machine (`docs/perf-exp/testbox_flood.sh`), 10 s per row:

| `server-unpack`, flood conns | opt-in req/s | opt-in p99 | flood req/s | flood refused |
|---|---:|---:|---:|---:|
| before this setting, none / 64 / 600 | 334,217 / 36,204 / 25,012 | 0.3 / 6.0 / 6.1 ms | — / 3,752 / 3,697 | 0 % |
| `always`, none / 64 / 600 | 306,896 / 28,633 / 25,067 | 0.3 / 7.3 / 6.0 ms | — / 3,730 / 3,896 | 0 % |
| `limited`, none / 64 / 600 | 307,399 / **236,300** / 19,014 | 0.3 / 0.8 / 7.4 ms | — / 1,529 / 146,805 | 0 / 96 % |
| `limited` without the queue (503 at once), 64 / 600 | 73,492 / 10,771 | 1.6 / 10.3 ms | 149,891 / 171,249 | 96 % |
| `never`, 64 / 600 | 105,186 / 19,185 | 0.7 / 5.4 ms | 230,883 / 308,022 | 100 % (406) |

(Load 1–9 from other users for the first three rows, 7–13 for the last two.) The flood does not starve the
blocking pool (opt-in p50 ≤ 1.4 ms with 600 waiting floods); it starves the CPU, every core transcoding. With the
queue, 64 flood connections all wait their turn on 4 slots: the flood gets 1.5k tiles/s, opt-in throughput stays
at 77 % of an idle server. At 600 the queue is full, refusals come back at once and the flood becomes ~150k cheap
requests per second from 600 client threads on the same box, which costs what any cheap flood does (`never` fares
no better): past that point it is a request rate, for a rate limit in front of the server, not this setting.

## Measured

Testbox (6-core Xeon E-2136), the 4096² world of docs/14 (16,641 hires tiles, 1.6 MB of PRBM each on average),
`serve_bench` + `load_bench`, every tile at most once, 5 s per row, server CPU per request:

| storage, client | 1 conn: CPU ms/req | p50 | 32 conns: req/s | KB sent/tile |
|---|---:|---:|---:|---:|
| compat, browser | 0.08 | 71 µs | 50,259 | 96 |
| optimized, PRBM as zstd | 3.04 | 2.2 ms | 1,621 | 92 |
| optimized, PRBM as gzip | 7.09 | 4.7 ms | 999 | 100 |
| optimized, client unpacks, zstd | **0.05** | 50 µs | **105,123** | **7** |
| optimized, client unpacks, gzip | 0.46 | 373 µs | 13,971 | 8 |

A tile served again from the transcode cache costs what compat does (0.09 against 0.08 ms) on any path.

Rerun at `957d8ad` (2026-10-28, one locked run at load 9–16, the docs/14 renders of that day), same order of rows:
CPU 0.09 / 3.04 / 7.04 / **0.06** / 0.45 ms, req/s 50,588 / 1,750 / 1,019 / **102,843** / 14,372, KB 96 / 92 / 100 /
**7** / 7: unchanged within noise (the sidecar index and decoder inlining since touch cold reads and the browser).

```
py -3 tools/testbox.py run <name> --sync --lock -- bash docs/perf-exp/testbox_web.sh ~/bmrs-<name> \
    <compat webroot> <optimized webroot of the same map>
```

The script first runs `tools/check_client_unpack.mjs`: the served client script in Node (WASM included) fetching
tiles through its `fetch` wrapper and comparing each with the compat webroot. 505 sampled tiles of that world were
identical, 8,272 B on the wire each (Node asks for gzip).

In a real browser (`tools/check_client_unpack_browser.mjs`, headless Chrome over the DevTools protocol, the same
map at x 0, z 0): the webapp's own 42 hires tile requests were all answered packed as zstd, 3,964 B on the wire
each; 40 of them refetched in the page were byte-identical to the PRBM the server makes for a client without the
script; the page logged no error or warning and drew the tiles.

```
node tools/check_client_unpack_browser.mjs "http://<host>:<port>/#<map>:0:70:0:60:0:0.9:0:0:perspective" \
    <chrome or edge exe> [screenshot.png]
```

Browser unpack time, workers and the hand-over to the webapp: docs/24.

## Limits

- nginx or `sql.php` still cannot serve an optimized storage: tiles live in bundle files.
- A tile whose position holds a NaN could unpack with a different NaN sign bit under WebAssembly's rules (not
  tested; a position is a finite block coordinate everywhere we have looked).
