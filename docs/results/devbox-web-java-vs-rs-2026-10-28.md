# Devbox: Java BlueMap's webserver vs ours, 2026-10-28

Java BlueMap's built-in webserver against `bluemap -w`. Both servers serve the same compat webroot, use the same
config, and are driven by the same client.

## Method

- **Machine:** devbox (Ryzen 7 9700X 8c/16t, 62 GB, NVMe), shared. All timed parts ran under `flock ~/bench.lock`.
  Busy CPUs from other users before each run were 1.1 / 3.4 / 1.2 of 16 (`BGLOAD`), and 8.1 after the last run. The
  1-minute load average, including our own client and server, was 6–12. Client and server run on the same box over
  loopback.
- **Versions:** Java is BlueMap 5.28 CLI (`work/downloads/bluemap-5.28-cli.jar`) on Temurin 25.0.4.1 with default JVM
  flags: G1, MaxHeap 14 GiB (¼ of RAM), 16 CPUs. Ours is bluemap-rs `eb1d73c` (synced as `a124561`),
  `target/release/bluemap`, glibc.
- **Webroot:** Java's 12-thread compat render of the 4096² world (`work/real/j12`), hard-linked into one scratch dir
  that both servers use as cwd. Both read the same `config/` and the same files: 16,641 hires `.prbm.gz` (94.2 KB
  mean) and 164 lowres PNGs (82.6 KB mean). Everything was in page cache.
- **Server config:** the generated default `webserver.conf`, unchanged except `ip: "127.0.0.1"`. Port 8100, default
  `additional-headers` (`Cache-Control: public, max-age=86400, …`), and the access log on (`data/logs/webserver.log`)
  for both servers.
  - Java runs a virtual thread per connection (52 OS threads seen).
  - Ours runs tokio with its default 16 workers plus a blocking pool (93–111 threads; 233 at 512 conns).
- **Load:** `docs/perf-exp/devbox_web_java_vs_rs.sh` (`load_bench` from `bm-web` examples, closed loop, keep-alive).
  - Every row restarts the server, warms it for 3 s at 32 conns, idles 3 s, then measures for 5 s.
  - Idle server CPU after the warmup was below 3 ms/s for both servers, so the CPU column has no background
    correction.
  - Server CPU is utime+stime from `/proc/<pid>/stat`. Peak RSS is `VmHWM`.
  - "Distinct" rows walk the shuffled list of all tiles. "Repeated" rows use the first 200 of that list.
  - KB/req is body bytes as `load_bench` counts them. At 1 conn Java covers only ~155 tiles per run, so its KB column
    there is a sample, not the mean.
- **Runs:** 3 runs, rows interleaved Java/ours, with the order flipped on each run. Tables show medians; ranges are
  in brackets where they matter.
- **Reload:** `docs/perf-exp/web_reload.py` loads one view over 6 connections, the browser's HTTP/1.1 limit.
  - The view is 75 URLs: `index.html`, both `settings.json`, `textures.json`, the 49 hires tiles nearest the median
    tile, and 9 lowres tiles per lod.
  - Full pass: 5 passes with no validators.
  - Reload pass: 20 passes sending `Cache-Control: no-cache` plus whatever `If-None-Match` / `If-Modified-Since` each
    URL's first reply allowed.
  - Wire bytes are everything read from the socket.

## Hires tiles, distinct

| `Accept-Encoding`, conns | server | req/s | p50 | p99 | server CPU ms/req | KB/req | peak RSS MB |
|---|---|---:|---:|---:|---:|---:|---:|
| `gzip`, 1 | Java | 31 | 41.0 ms | 42.0 ms | 0.77 (0.76–1.02) | (sample) | 183 |
| | ours | 32,529 | 27 µs | 76 µs | **0.030** | 94.1 | 88 |
| `gzip, deflate, br, zstd`, 1 | Java | 32 | 41.0 ms | 42.3 ms | 0.95 (0.56–1.17) | (sample) | 185 |
| | ours | 33,826 | 26 µs | 74 µs | **0.029** | 94.1 | 86 |
| `gzip`, 32 | Java | 1,011 | 41.0 ms | 42.0 ms | 0.31 | 92.5 | 308 |
| | ours | 122,897 (117–146k) | 212 µs | 891 µs | **0.070** | 94.2 | 104 |
| `gzip, deflate, br, zstd`, 32 | Java | 1,014 | 41.0 ms | 42.1 ms | 0.36 | 92.7 | 319 |
| | ours | 127,661 (101–153k) | 207 µs | 835 µs | **0.070** | 94.2 | 110 |
| `gzip, deflate, br, zstd`, 512 | Java | 52,456 (16–65k) | 430 µs | 43.9 ms | 0.135 | 96.2 | 716 |
| | ours | 73,746 (69–84k) | 6.0 ms | 20.8 ms | 0.111 | 94.1 | 145 |
| `gzip;q=1.0`, 1 | Java | 27 | 42.0 ms | 49.4 ms | 3.65 | **1,888** | 414 |
| | ours | 26,636 | 31 µs | 110 µs | 0.037 | 94.2 | 89 |
| `gzip;q=1.0`, 32 | Java | 946 | 42.0 ms | 49.0 ms | 2.04 | **1,591** | 555 |
| | ours | 130,193 | 204 µs | 795 µs | 0.070 | 94.2 | 112 |

At 512 conns the client is the limit: 512 client threads share 16 CPUs with the server. Both servers had 2 failed
connects per run there; every other row had none.

## Repeated (200 tiles), lowres, JSON

| row | server | req/s | p50 | server CPU ms/req | KB/req | peak RSS MB |
|---|---|---:|---:|---:|---:|---:|
| hires 200 repeated, 1 conn | Java | 31 | 41.0 ms | 0.83 | 106 | 188 |
| | ours | 26,095 | 30 µs | 0.038 | 114 | 80 |
| hires 200 repeated, 32 conns | Java | 1,007 | 41.0 ms | 0.31 | 114 | 324 |
| | ours | 107,586 | 245 µs | 0.085 | 114 | 90 |
| lowres PNG, 1 conn | Java | 26 | 41.0 ms | 0.89 | 83 | 173 |
| | ours | 31,532 | 22 µs | 0.032 | 83 | 69 |
| lowres PNG, 32 conns | Java | 858 | 41.0 ms | 0.35 | 83 | 321 |
| | ours | 150,878 (66–165k) | 174 µs | 0.056 | 83 | 70 |
| map `settings.json` + `textures.json`, 1 conn | Java | 26 | 41.0 ms | 0.89 | 285 | 220 |
| | ours | 17,946 | 64 µs | 0.057 | 287 | 126 |

Neither server caches compat data. Repeated tiles cost what distinct ones do: the 200-tile set is just larger tiles
on average.

## Reload revalidation (one view, 75 requests, 6 connections)

| pass | server | validators sent | status | wall (median) | wire per view | hires wire/tile | server CPU ms/req |
|---|---|---|---|---:|---:|---:|---:|
| full load | Java | – | 75 × 200 | 461 ms | 5,785 KB | 58.2 KB | 1.60 |
| | ours | – | 75 × 200 | 1.2 ms | 5,748 KB | 57.9 KB | 0.027 |
| reload | Java | INM+IMS on 2 static files, none on 73 map URLs | 73 × 200, 2 × 304 | 454 ms | **5,782 KB** | 58.2 KB | 0.65 |
| | ours | INM on all 75 (+IMS on the 2 static) | **75 × 304** | 0.8 ms | **13.4 KB** | 0.18 KB | 0.020 |

Java sends no `ETag` or `Last-Modified` on map data (tiles, lowres, map JSON). A revalidating reload therefore
downloads the whole view again: 432× the bytes ours sends. Java's full-load wire bytes run 0.6% above ours, from
1 KiB chunked framing on identical bodies.

## Findings

1. **Java stalls ~41 ms on every response after the first on a keep-alive connection.** p50 is 41.0 ms in every
   Java row at 1 and 32 conns, so throughput is conns / 41 ms (≈ 31 at 1, ≈ 1,000 at 32).
   - Traced with `strace` on 5.28: the connection socket never gets `TCP_NODELAY`. The 225-byte header is written on
     its own, then the body follows in ~7.2 KB writes. That is Nagle against the client's delayed ACK (docs/11 §2.4).
     With curl, the first tile on a connection took 6.7 ms and the second 46.7 ms.
   - Loopback's 64 KB MSS makes this worst case. Over a real network with 1448-byte segments most of the body goes out
     in full segments, and the stall would hit mainly the trailing partial segment, so how much a real browser loses
     depends on RTT and the client OS.
   - In this setup it decides the page load: one 75-URL view takes 461 ms from Java against 1.2 ms from ours.
2. **CPU per tile** (the part loopback does not distort):
   - 1 conn: Java 0.77–0.95 ms, ours 0.030 ms (26–33×).
   - 32 conns: Java 0.31–0.36 ms, ours 0.070 ms (4.5–5×).
   - 512 conns: Java 0.135 ms, ours 0.111 ms (1.2×). Java's cost per request falls as load rises; at 512 conns it is
     close to ours.
   - Lowres and JSON follow the same pattern (6–16×).
3. **Throughput with the stall hidden:** at 512 conns, where the client is the limit, Java reached 52k req/s (16–65k,
   unstable) against ours at 74k (69–84k). Java's p99 there stays at 44 ms, so some requests still hit the stall.
4. **The docs/11 q-value bug is still in 5.28.** `Accept-Encoding: gzip;q=1.0` (and `gzip;q=1.0, identity;q=0.5`)
   gets identity PRBM from Java:
   - 1.59–1.89 MB per tile against 94 KB, about 17–20×.
   - 2.0–3.65 ms CPU per tile and 555 MB RSS.

   `gzip` and the real browser header `gzip, deflate, br, zstd` both get the stored gzip passed through, on both
   servers. Ours parses q-values: the same gzip bytes for all four headers.
5. **Memory:** Java's peak RSS was 173–324 MB at 1–32 conns, 555 MB with the identity bug and 716 MB at 512 conns.
   Ours was 69–112 MB (145 at 512): 2–5× less.
6. **Revalidation:** see the reload table. Ours answers a no-cache reload with 75 × 304 (13 KB). Java answers it with
   73 full 200s (5.8 MB). Both send the default `Cache-Control: max-age=86400`, so the two servers behave the same
   within 24 h when the browser does not revalidate.
7. **Noise:** ours at 32 conns varied 100–153k req/s across runs, with other users' load (run 2 had 3.4 busy CPUs from
   others). Java's 1–32 conn rows barely moved because they are bound by the stall, not by CPU.

## Reproduce

```
MSYS_NO_PATHCONV=1 BM_TESTBOX=devbox py -3 tools/testbox.py run devbench --lock --detach --job web -- \
  bash docs/perf-exp/devbox_web_java_vs_rs.sh '~/bmrs-devbench' '~/bmrs-devbench/work/real/j12' \
  '~/bmrs-devbench/work/downloads/jdk25/bin/java' '~/bmrs-devbench/work/downloads/bluemap-5.28-cli.jar' 3 5
```

- The command needs `target/release/bluemap` and `target/profiling/examples/load_bench` built in that checkout.
- The scripts must be in the remote checkout: sync it, or copy them there.
- Raw rows go to `~/jvr-web/rows.jsonl`. This run's are kept as `~/jvr-web/rows-final.jsonl` on the devbox. About
  15 min.
