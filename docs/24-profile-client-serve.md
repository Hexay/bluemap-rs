# 24 — Profile: client-side unpacking and serving of optimized hires tiles

Scope: what a browser viewing an `optimized` file storage pays per hires tile (docs/18), and what bm-web pays to
send it. Measured 2026-10-22 on the 155H laptop (P-cores pinned, Chrome 155, Node 22) and, unlocked, on the
testbox (load average 50–160 from other jobs: its numbers are profiles and ratios only).

## Harness (`crates/bm-wasm/bench/`)

```
cargo run --release -p bm-wasm --example corpus -- <dir of x…/z….prbm.gz> <corpus dir> [--step 4]
cargo run --release -p bm-wasm --example unpack_bench -- <corpus dir>          # native baseline
node crates/bm-wasm/bench/node.mjs <corpus dir> [unpack.wasm] [reps] [--frames]   # WASM in Node (--frames: from zstd)
node crates/bm-wasm/bench/browser.mjs <corpus dir> <chrome> [unpack.wasm]        # WASM in Chrome
node crates/bm-wasm/bench/browser.mjs <corpus> <chrome> <wasm> 3 --client crates/bm-web/client/unpack.js [--pipeline '{"tiles":48}']
node crates/bm-wasm/bench/view.mjs "<map url>" <chrome> [--script js --wasm wasm | --no-script] [--net 20,50]
bash crates/bm-wasm/bench/native_perf.sh <corpus dir>                            # Linux perf of the decoder
```

- Corpus: 256 tiles of the 4096² world (`work/exp/real`, every 4th), 1.93 M quads, 1.31 MB of PRBM per tile.
- `browser.mjs` serves the page cross-origin isolated (5 µs timers); `--client` runs the shipped script against a
  `fetch` that answers with packed bodies, then the webapp's `RevalidatingFileLoader` steps, 8 tiles in flight like
  its `TileManager`; main-thread time is `Performance.getMetrics` `TaskDuration`.
- `view.mjs` drives the real webapp until its hires tile manager has every tile of the view; the client script is
  swapped by DevTools `Fetch` interception, so one server serves every variant. Do not pass `--net` with a huge
  throughput: Chrome then stalls every request.

## Client

### Where a tile's time went (before)

Chrome, 256 tiles, warm: the old script spent **4.2 ms of main thread per tile** (6.4 cold): 0.75–0.9 unpack,
0.45 copying the PRBM out of WASM memory, and **~2.2 ms in the webapp's loader** re-streaming a `Response` built from
1.3 MB of bytes (it reads `response.body` chunk by chunk for progress, builds a second `Response`, then
`arrayBuffer()`). Strategies, same harness (ms per tile, main thread / wall):

| hand-over | main | wall |
|---|---:|---:|
| main thread, `new Response(bytes)` (old) | 4.20 | 4.20 |
| main thread, `Response(ReadableStream)` | 3.05 | 3.05 |
| 2 workers, stream | 1.46 | 1.45 |
| main thread, no body stream | 1.37 | 1.35 |
| 2 workers, no body stream | 0.20 | 0.88 |
| 4 workers, no body stream | 0.23 | 0.63 |

### What changed (`crates/bm-web/client/unpack.js`)

- Unpacking runs in up to 4 workers (`hardwareConcurrency - 2`, at least 1), started from a Blob URL with the module
  compiled once on the main thread and posted to each. The packed body is transferred in, the PRBM transferred back.
  No `Worker`, a module that cannot be posted, or a worker that fails to start (CSP without `blob:`) falls back to
  the main thread; a failed tile still falls back to a plain PRBM request.
- The webapp gets an `Unpacked` `Response` whose `body` is `undefined` and whose `arrayBuffer()` returns the PRBM
  buffer itself: the loader's `body.getReader() === undefined` branch, no re-streaming, no copy on the main thread.
  `blob()`, `text()`, `bytes()`, `clone()` work from the same bytes.
- A decoder whose WASM memory grew past 64 MB (a huge tile) is re-instantiated after that tile; memory never shrinks.
- The script tag is `defer` (it downloaded and ran before the parser reached the webapp's module script, a round
  trip in series); deferred and module scripts run in document order, so it still wraps `fetch` first. The script's
  URL is hashed, so it is now sent `Cache-Control: public, max-age=31536000, immutable`.

### Results

Shipped script, Chrome, `--client` (ms per tile):

| | main, warm | wall, warm | main, cold | wall, cold |
|---|---:|---:|---:|---:|
| 256 tiles, old | 3.95–4.35 | 3.93–4.24 | 7.3–8.4 | 7.2–8.4 |
| 256 tiles, new | 0.61–0.63 | 0.79–0.80 | 2.7–3.4 | 2.7–3.4 |
| one 48-tile view, old | 4.2–5.1 | 4.1–4.9 | 4.5–5.4 | 4.4–5.3 |
| one 48-tile view, new | 0.28–0.31 | 0.40–0.45 | 0.62–0.64 | 0.89–0.98 |

Node (no workers, main-thread fallback): 3.5 → 1.6 ms per tile.

Real webapp, `view.mjs`, a 42-tile view of the same real-world tiles served by `bluemap -w` from an optimized storage,
fresh profile per run, median of 3 (headless, SwiftShader, so the webapp's own GL work is on the CPU too):

| tiles from | loopback: view ms | main ms | 20 ms RTT, 50 Mbit/s: view ms |
|---|---:|---:|---:|
| server PRBM (no script) | 3195 | 2179 | 4679 |
| old script | 3310 | 2268 | 4490 |
| new script | **2775** | **1741** | **3889** |

The old script was slower than plain PRBM on loopback (its main-thread unpack queued behind the webapp's own work);
the new one is 16% faster than the old on loopback and 13% on the throttled link (17% faster than PRBM there).
Longest task unchanged (~200–400 ms: the webapp's start-up, not tiles).

`tools/check_client_unpack_browser.mjs` with the new script overlaid on the served one: 42 of 42 tiles packed, 40
refetched byte-identical, no page error. `tools/check_client_unpack.mjs` (Node): 342 tiles identical.

Memory: each decoder peaks at 9.4 MB of WASM memory on this corpus (1.1 MB initial), so 4 workers hold ~38 MB
where the old script held 9.4 MB on the main thread, plus each worker's isolate.

## Decoder (`bm_format::compact`, decode side)

Profile in Node (`--cpu-prof`, a build with the name section): the wasm build left `array::map` closures as calls in
the per-quad loop (`normal_bytes`' three maps, ao `level` and row maps, `Grid::predict`), 20% of decode time. Native
x86 did too (`perf` on testbox: `normal_bytes` closures and the `predict` closure as their own symbols). They are
array literals now (`prbm::each3`, `decode.rs`, `ao.rs`); `Probes::predict`'s per-vertex closure stays one call
(inlining it made wasm 18% slower). Output bytes unchanged (corpus FNV check in every harness, `cargo test -p
bm-format`).

| ns/quad | before | after |
|---|---:|---:|
| native (laptop, P-core) | 84–90 | 75–76 |
| WASM, Node | 113–115 | 92–93 |
| WASM, Chrome | 174 | 144 |

Tried, no gain: `+simd128` (no auto-vectorised loop; +3 KB), `-C target-cpu=mvp` (−8%: bulk memory helps),
`opt-level=s` (2.8× slower), higher LLVM inline thresholds, memoising the column division in `cells::decode`.
Not tried: wasm-opt (no binaryen here). Remaining per-quad split in WASM: main loop 57%, `predict` 20%, cells 9%,
attribute headers and payload zero-fill 7%. Copy-out of the PRBM (0.45 ms per tile) now runs in the worker.

## Server (bm-web)

Unlocked testbox profiles, `serve_bench` + `load_bench`, 8 connections, every tile of the 4096² world once:

- Packed tile as zstd (the browser path): 16.9 k req/s at 0.079 ms CPU per request on the loaded box, 64 MB peak
  RSS. The profile is flat: kernel send/recv, hyper, `malloc`; 2.3% xxh32 verifying the record. Nothing left here
  worth a change.
- PRBM as zstd (clients without the script): 6.9 ms CPU per request on the loaded box; `ZSTD_compressBlock_doubleFast`
  + `ZSTD_encodeSequences` 52%, decode 17%; 199 MB peak RSS at 8 connections.
- **Cold page cache**: the same packed run right after the bundles had been evicted did 109 req/s with p99 1.45 s,
  19% of CPU in xxh32. See "Storage" below.

A locked rerun of `docs/perf-exp/testbox_web.sh` on this branch is queued behind the bench lock
(`py -3 tools/testbox.py wait websrv`).

## Storage read path (bm-storage, not changed here)

- **First read of a bundle reads and checksums the whole bundle** (`BundleStore::refresh` → `log::scan` over
  `read_range(ix.end, len)`): 1.3–2.6 MB of I/O and xxh32 to serve one ~7 KB tile. At 300k² blocks there are
  ~340 k bundles (~700 GB), far beyond the page cache, so a crowd roaming the map pays this on most first visits
  to an area. The index cache (`CACHE_CAP` 4096 bundles) is cleared wholesale when full, so past 4096 distinct
  bundles every bundle is rescanned again. Options: an index (footer or sidecar) per bundle, scanning record heads
  only (a torn tail can only be the last record; `read_record` verifies the record it serves anyway), an LRU
  instead of clear-all.
- The per-bundle mutex is held across `metadata`, the header read and the record `pread`: concurrent readers of one
  bundle (every viewer of the same area) are serialised on file I/O.
