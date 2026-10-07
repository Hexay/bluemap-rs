# bm-web serving profile (2026-10-07)

Measured, not static reading. Complements `docs/11-perf-audit-storage-web.md` (Java audit); item numbers `#n` refer to its "Ranked wins".

## Setup
- `crates/bm-web/examples/serve_bench.rs`: serves a webroot, every `maps/<id>` as read-only gzip `FileStorage`, `--live` (players.json with 20 players pushed every 1 s, markers, SSE), `--log <file>` (access log), `--exit-after`. Counting global allocator; `stats` on stdin prints allocs/bytes/peak-live and resets.
- `crates/bm-web/examples/load_bench.rs`: closed-loop keep-alive HTTP/1.1 client (thread per connection), p50/p90/p99/max, `--sse N` holds N event streams.
- `crates/bm-web/examples/web_components.rs`: µs/op of the per-request building blocks (fs ops, codecs, `spawn_blocking`).
- `docs/perf-exp/web_bench.py`: runs the scenario table below; server CPU (user+kernel) and working set via psutil.
- Fixture `work/bluemap/structures/web` (1154 hires tiles, avg 70 KB gz; 67 lowres PNGs). 22-thread Windows 11 box, loopback, client on the same machine, `profiling` build. Browser `Accept-Encoding: gzip, deflate, br, zstd` unless marked identity. 32 connections, 5 s each, one run.
- **No sampling profile.** samply on Windows needs admin (ETW); the UAC prompt went unanswered. `docs/perf-exp/web_profile.py` is ready (one UAC prompt, MSVC-map symbols via `bluemap_reverse/tools/profile.py`). Attribution below is by differential scenarios + `web_components`.

```
cargo build -p bm-web --profile profiling --examples
py -3 docs/perf-exp/web_bench.py work/bluemap/structures/web --secs 5 --conns 32 [--conns 1] [--log <file>]
target/profiling/examples/web_components.exe work/bluemap/structures/web structures
```

## Numbers (32 conns)

| scenario | req/s | MB/s | p50 µs | p99 µs | CPU ms/req (kernel) | allocs/req | KB alloc/req |
|---|---|---|---|---|---|---|---|
| floor: 404, no I/O (`maps/<id>/nothing`) | 42 124 | 0 | 594 | 4 267 | 0.18 (38%) | 38 | 12.5 |
| missing hires tile → 204 | 31 219 | 0 | 810 | 5 534 | 0.36 (49%) | 57 | 13.7 |
| hires `.prbm` gzip passthrough | 21 022 | 1 477 | 1 031 | 8 964 | 0.56 (67%) | 60 | 82 |
| hires identity (gunzip per req) | 1 704 | 2 118 | 11 442 | 109 471 | **7.90** (51%) | 74 | **3 694** |
| lowres PNG | 24 426 | 305 | 943 | 7 396 | 0.43 (63%) | 59 | 26 |
| map `settings.json` (354 B, re-gzipped per req) | 7 654 | 1.9 | 3 553 | 22 816 | 0.69 (60%) | 53 | **418** |
| `textures.json` gzip passthrough (588 KB) | 3 837 | 2 256 | 5 365 | 35 534 | 1.71 (82%) | 47 | 588 |
| `textures.json` identity (2.5 MB gunzip) | 848 | 2 120 | 26 592 | 152 275 | **17.2** (39%) | 63 | **8 858** |
| static JS 1.2 MB (identity, streamed) | 1 770 | 2 128 | 13 782 | 61 948 | **7.03** (63%) | 99 | 148 |
| static small (index, settings, lang, css) | 6 933 | 37 | 4 731 | 28 543 | **1.23** (65%) | 81 | **143** |
| static → 304 (`If-Modified-Since`) | 7 754 | 0 | 4 134 | 28 942 | **1.00** (66%) | 67 | 15 |
| one-view mix (8 static/meta + 49 hires + 243 lowres) | 22 783 | 417 | 912 | 8 499 | 0.49 (58%) | 58 | 31 |
| live `players.json` (4.4 KB, in memory) | 39 658 | 181 | 650 | 4 232 | 0.18 (40%) | 41 | 14 |

- 1 connection (pure latency, p50): floor 75 µs, 204 tile 204 µs, hires 378 µs, lowres 286 µs, static 304 **569 µs**, static small 658 µs, JS 2.9 ms, hires identity 3.0 ms.
- Access log on (`--log`): floor +0.11 ms CPU/req (+60%), +26 allocs/req; hires −20% req/s.
- SSE, 1000 clients, players changing every 1 s: 145 ms CPU/s, 4.9 KB/s **per client** (17.6 MB/h per open tab, uncompressed), 5.5 allocs per client-event, 23 MB peak heap.
- Memory: idle 6 MB working set; 17 MB peak serving gzip passthrough; 91 MB with 32 concurrent identity hires; **123 MB** with 32 concurrent identity `textures.json`. Scales with concurrency × decoded size; nothing bounds it (512 blocking threads, `MAX_DECODED` = 1 GiB).
- Bytes on the wire, one cold default view of `structures` (measured): 5.49 MB = hires 2.83 MB (49) + static 1.23 MB (JS 1.20 MB identity; gzip-6 = 0.31 MB) + lowres 0.84 MB (67 + 176×204) + textures 0.59 MB. Map data carries no `ETag`/`Last-Modified`, so every `no-cache` reload re-sends ~4.3 MB.

`web_components` (µs/op, one thread): `fs::read` 40 KB tile 137; `fs::read` missing 29; `fs::metadata` 38 (missing 50); `File::open` 70; `tokio::fs::metadata` **110**; `fs::read` 1.2 MB 687; `spawn_blocking` no-op 11 sequential / 4.6 under load; gzip-6 of 354 B **110** (deflate state setup); gunzip tile 667 KB raw 1 572; gunzip textures 2.5 MB raw **11 889**; gzip-6 tile raw 14 350.

## Ranked hotspots

1. **Static files: 3 blocking fs hops before anything, including 304s** — `static_files.rs:45` (`is_dir` → `tokio::fs::metadata`), `:57-62`/`:102-111` (`metadata` + `File::open` per candidate, `index.html` fallback doubles it), 304 check only at `:72` after the open. Each `tokio::fs` call is a `spawn_blocking` (110 µs vs 38 µs std). A 304 costs 1.0 ms CPU, 5.5× the floor and **more than a full 70 KB hires tile**. `:77` allocates a fresh 64 KiB `ReaderStream` buffer (+ tokio `File` buffer) per response → 143 KB alloc for a 1 KB file; the 1.2 MB JS takes ~19 blocking hops (7 ms CPU vs 2.9 ms/MB for an in-memory body). Static bodies never compressed (#2).
2. **No validators / no cache on map data** — `map_handler.rs:91-107`: every tile/item is `spawn_blocking` + `fs::read` (`bm-storage/src/file/fsops.rs:59`) and a 200 without `ETag`/`Last-Modified`. File read ≈ 140 µs of the 0.56 ms hires cost; reloads re-send everything (#1).
3. **Per-request transcoding without caching** — `encoding.rs:57-58` re-gzips NONE-stored items (`settings.json`: 110 µs + 418 KB alloc to save 107 B); `:60` gunzips for identity clients (textures 17 ms CPU, 8.9 MB alloc per request). `bm-compress/src/lib.rs:157` `read_to_end` without a size hint → doubling reallocs (3.7 MB alloc for a 0.67 MB tile). Unbounded concurrency makes this the RSS driver.
4. **Fixed per-request overhead: 38 allocs, 0.18 ms on a no-I/O 404** — `app.rs:145-151`: `decode_path` String, `java_query_string` Vec+Strings, `format!("{path}?{query}")` + `request_info` (`method.to_string()`, `forwarded_for`) built even when the log is disabled; `server.rs:93-94` `PeerAddr` extension insert (allocates the extensions map) per request; `encoding.rs:14-29` `Accepted` = `Vec<String>` with a `to_ascii_lowercase` per coding (4 allocs for a browser header).
5. **Access log** — `access_log.rs:68-78` clones every arg into a fresh `String`; `:135` `chrono::Local::now()` per line (timezone lookup on Windows); `:98/:136` `Mutex` around an `mpsc::Sender` that is already `Sync`. +60% CPU on small requests; on by default in BlueMap's `webserver.conf`.
6. **Live/SSE bytes** — `live.rs:87-93` sends the full players payload uncompressed per change (4.9 KB/s per tab for 20 players); `:117` re-arms a `tokio::time::timeout` per event per client.

Not hot: tile-path parsing (`map_handler.rs:134`), routing loop (`app.rs:126`), `LiveJson` mutex, header finishing. Kernel time (loopback send, file opens with Defender) is 40–80% of server CPU; the fixes above cut syscalls, not just user time.

## Fix proposals (all C = drop-in compatible)

| # | Fix | Where | Expected gain |
|---|---|---|---|
| A | In-memory static cache: `(path) → {Bytes, gz/br variants, mtime, len, etag}`; one `std::fs::metadata` inside a single `spawn_blocking` (or a 1–2 s revalidation TTL) instead of `metadata`+`open`+stream; check `If-None-Match`/`If-Modified-Since` before opening; serve unchanged bundled files straight from `rust-embed` | `static_files.rs` | 304: 1.0 → ~0.2–0.25 ms CPU (4–5×); small static 1.2 → ~0.25 ms, 143 → <15 KB alloc; JS 7 → ~1 ms CPU |
| B | Precompressed webapp assets (gzip-9 + br-11 at build or first hit), negotiated with q-values, `Vary: Accept-Encoding` | `static_files.rs`, `webapp.rs` | −0.92 MB (gz) / −0.95 MB (br) per cold visit; JS CPU another ~4× lower |
| C | `ETag` (strong, from mtime+len for files, content hash for SQL) + `Last-Modified` on tiles/items; 304 on match | `map_handler.rs` | Reload: ~4.3 MB → ~60 KB per view (#1); a 304 costs ≈ one `metadata` (38 µs) instead of `read` (137 µs) |
| D | Cache encoded variants of small/NONE-stored items (`settings.json`, assets) keyed by mtime/len; skip gzip below ~1 KB | `encoding.rs:57` | settings.json 0.69 → ~0.36 ms, 418 → ~14 KB alloc/req |
| E | Pre-size decompression from the gzip ISIZE trailer (`decompress_into` with `reserve`); cap concurrent transcodes with a `Semaphore` (e.g. = cores) | `bm-compress/src/lib.rs:156`, `map_handler.rs:91` | Identity alloc/req −50–60%; peak RSS bounded (today 123 MB at 32 conns, unbounded beyond) |
| F | Optional hot-tile cache (LRU of `Bytes`, ~64 MB, invalidated by in-process writes; mtime check for external writers) | `map_handler.rs` / `bm-storage` | hires 0.56 → ~0.35 ms CPU (−35–40%), p50 −30% |
| G | Per-request alloc diet: build `RequestInfo`/address only when a sink exists; `Accepted` as a bitset over known codings; `Cow` paths when nothing is percent-encoded; peer addr in a per-connection service instead of `Extensions` | `app.rs:145-176`, `encoding.rs:11-34`, `server.rs:93` | 38 → ~15 allocs on the floor; ~10–20% CPU on 204/lowres-sized requests |
| H | Access log: format straight into one `String`, cache the second-resolution timestamp, drop the `Mutex` | `access_log.rs:64-81,133-139` | Log overhead 0.11 → ~0.02 ms/req |
| I | SSE/live: gzip `players.json`/`markers.json` once per change (cache), optional per-connection gzip stream with sync flush; round doubles to 3 dp | `live.rs` | ~3–5× fewer live bytes (17.6 MB/h per tab → ~4 MB/h, estimate) |

Order by value: C and B (bytes, every user), A (CPU+latency on every page load and every 304), D/E (CPU+RSS spikes from non-browser clients), then F/G/H/I.
