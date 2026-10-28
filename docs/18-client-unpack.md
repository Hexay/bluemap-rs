# 18 — Client-side unpacking of optimized hires tiles

An `optimized` storage keeps hires tiles as BMQ3 blobs (docs/17), about 12× smaller than the gzip PRBM the webapp
loads. Serving one to the unmodified webapp means unpacking it and compressing 1–2 MB of PRBM again: ~3 ms of
server CPU per tile nobody asked for recently, against 0.08 ms for a compat tile (numbers below). With client-side
unpacking the server sends the blob's content as stored and the browser rebuilds the PRBM.

It is negotiated per request. A client that does not ask gets PRBM exactly as before.

## Wire

- **Request**: a hires tile URL (`maps/<id>/tiles/0/x…/z….prbm`) with `Accept` naming
  `application/vnd.bluemap.bmq3`.
- **Reply**, if the storage packs hires tiles and the tile is stored in model mode: `Content-Type:
  application/vnd.bluemap.bmq3`, body = the tile's *model body* (docs/17, `bm_format::compact`):
  - client accepts zstd: the blob's zstd frame, byte for byte from storage, `Content-Encoding: zstd`;
  - else gzip: the frame decompressed and gzipped (transcode cache, ~8 KB per tile);
  - else identity.
- A tile stored raw (the model could not hold it), a `.gz` URL, a compat storage or a server with the feature off
  answers with PRBM and `application/octet-stream`. The client tells the two apart by `Content-Type`.
- `Vary: Accept, Accept-Encoding` on every hires reply of a packing storage while the feature is on.
- ETags: `"<version>-bmq3-zstd"`, `-bmq3-gzip`, `-bmq3`, distinct from every PRBM representation's. A revalidation
  from an unpacking client matches the packed tag or the PRBM tag (it holds whichever form the tile had).

Code: `crates/bm-web/src/client_unpack.rs`, `map_data::serve`, `transcode::packed_body`,
`bm_storage::packed_model_frame`.

## Client

`crates/bm-web/client/unpack.js`, with the WASM unpacker (`crates/bm-wasm`, 46 KB) inlined as base64. The webapp
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

## Clients that don't unpack

`webserver.conf` `server-unpack` (generated with a comment; absent = `limited`, so Java configs load unchanged)
decides what a request without the opt-in (or a `.gz` URL) gets for a packed hires tile:

- `limited`: unpacked by the server, at most max(1, cores / 4) at once (`WebOptions::server_unpack_slots`). With
  none free: `503`, `Retry-After: 1`, `no-store`, at once.
- `never`: `406` with a one-line `text/plain` body. Tiles stored raw are still served, under the `limited` slots.
- `always`: every request unpacked, bounded only by the transcode memory gate (cores at once, waiting).

A transcode-cache hit and an `If-None-Match` 304 cost no slot; opt-in requests, raw tiles for them, compat
storages and everything but hires tiles never touch the setting. The slot is taken without waiting
(`Gate::try_acquire`) inside the blocking task, after the tile is read: that read is what an opt-in request costs
anyway, and deciding there lets cache hits, 304s and `never`'s raw tiles through exactly. Slot holders skip the
memory gate (the slots bound them tighter); all other transcodes (items, gzip model bodies) keep it.

The webapp does not retry a failed tile until it leaves the view distance, so a refused viewer without the script
sees a hole: serve `index.html` from bluemap-rs (or proxy it) and keep `client-unpack` on, else use `always`.

Devbox (Ryzen 9700X, 16 threads), the 4096² world, 2026-10-28, one locked run at load 7–13 from other users:
32 opt-in connections on one half of the tiles, alone and with a closed-loop flood of the other half without the
opt-in from the same machine (`docs/perf-exp/testbox_flood.sh`), 10 s per row:

| `server-unpack`, flood conns | opt-in req/s | opt-in p99 | flood req/s | flood refused |
|---|---:|---:|---:|---:|
| before this setting, none | 334,217 | 306 µs | | |
| before this setting, 64 | 36,204 | 6.0 ms | 3,752 | 0 % |
| before this setting, 600 | 25,012 | 6.1 ms | 3,697 | 0 % |
| `always`, 64 / 600 | 18,953 / 15,586 | 11.0 / 10.0 ms | 2,806 / 2,876 | 0 % |
| `limited`, none / 64 / 600 | 288,412 / 73,492 / 10,771 | 0.4 / 1.6 / 10.3 ms | — / 149,891 / 171,249 | 96 % |
| `never`, 64 / 600 | 105,186 / 19,185 | 0.7 / 5.4 ms | 230,883 / 308,022 | 100 % (406) |

The flood does not starve the blocking pool (opt-in p50 stayed ≤ 1.4 ms with 600 waiting floods); it starves the
CPU, every core transcoding. `limited` takes that away at 64 connections (4× the opt-in throughput of `always`).
At 600 the refusals come back so fast that the flood becomes ~170k cheap requests per second from 600 client
threads on the same box, which costs as much as any cheap flood (`never` fares little better): past that point it
is a request rate, for a rate limit in front of the server, not something this setting can bound.

The unpacker is `bm_format::compact::BodyUnpacker`, the same code the server runs, built without the `codec`
feature (no zstd: the browser has already undone the content coding). WebAssembly float arithmetic is IEEE 754, so
positions and normals come out bit-identical. After changing the decoder or `bm-wasm`, rebuild the checked-in
module: `py -3 tools/build_wasm.py`.

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
