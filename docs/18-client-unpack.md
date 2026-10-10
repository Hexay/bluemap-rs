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
header to hires tile requests, and hands the webapp a `Response` holding the unpacked PRBM. Any failure (WASM
unavailable, corrupt body) falls back to an ordinary request.

- The server adds `<script src="./assets/bluemap-rs-unpack-<hash>.js">` in front of the webapp's module script
  when it serves `index.html`, and serves the script at that URL. Both only while a registered map's storage packs
  hires tiles; the files in the webroot are never touched. An `index.html` that already names the script is left
  alone.
- An `index.html` served by something else (nginx in front of the webroot) does not get the tag, so those viewers
  stay on the PRBM path.
- A viewer opts out with `localStorage.setItem("bluemap-rs-client-unpack", "off")` and a reload.
- A server opts out with the hidden `webserver.conf` key `client-unpack: false`.

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

Not measured: the browser's unpack time per tile (natively the same code takes ~1 ms for these tiles).

## Limits

- nginx or `sql.php` still cannot serve an optimized storage: tiles live in bundle files.
- A tile whose position holds a NaN could unpack with a different NaN sign bit under WebAssembly's rules (not
  tested; a position is a finite block coordinate everywhere we have looked).
