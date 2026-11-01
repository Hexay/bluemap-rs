# Faster webapp loads for script clients (2026-11-01)

Optimized storage + client script. Headless Chrome on the devbox, GL draws off, `mss_relay.py` + CDP shaping, fresh profile,
4096² world, 42-tile view. Same server build for all rows; the script is swapped by DevTools, interleaved, 5 runs per row,
median (min-max), ms. Headless numbers are lower than the real-GPU ones (1.7/2.9 s first load) and say nothing about frame
pacing. Regenerate: `docs/perf-exp/devbox_load_ab.sh`, report `crates/bm-wasm/bench/load_ab_report.mjs`.

## Where a reload's time went

Traced reload of the old script (`LOAD_ARGS=--trace+--no-pan`, 5 runs; tracing slows it). Until the view is in:

| | 20 ms, 50 Mbit | 50 ms, 10 Mbit |
|---|---:|---:|
| time to view | 1029 | 2232 |
| revisit in the same profile, no revalidation | 486 | 566 |
| **reload minus revisit** | **543** | **1666** |
| wall: main thread busy | 748 | 1220 |
| wall: main idle, waiting on the network | 273 | 1008 |
| wall: workers only / nothing | 0 / 0 | 0 / 0 |
| main thread split: script / GL calls / parse, compile / other | 324 / 91 / 27 / 157 | 460 / 151 / 29 / 215 |
| unpack workers, busy (parallel) | 23 | 22 |
| GPU process main, busy (no draws; not a real-GPU figure) | 98 | 156 |
| hires: requests, all conditional | 42 | 42 |
| hires: queue before send, median / sum | 87 / 3682 | 245 / 10142 |
| hires: RTT, median / sum | 22 / 923 | 52 / 2179 |
| lowres requests before the view | 173 | 189 |
| lowres: queue before send, median | 88 | 240 |
| bytes, all phases | 25 KB | 25 KB |

- **Largest cost: queueing for a connection.** About 260 requests of one round trip each share 6 connections. A hires tile
  waits 4-5 RTTs for a socket, behind lowres revalidations that use the same ones. Main-thread time grows with it (more
  responses to handle) but it is not the cause: a revisit does the same rebuild in 486/566 ms.
- GPU upload and draw were not measured: draws are off here, and the trace of a real GPU is not part of this run.
- Unpacking is negligible (23 ms).

Real BlueMap: `Cache-Control: public, max-age=86400, stale-if-error=604800` from the default `additional-headers`, and no
validators on map data, so the same reload re-downloads everything (docs/results/devbox-webapp-load-breakdown-2026-11-01.md).

Why a short cache lifetime can't help: the webapp itself sets `cache: "no-cache"`, only on a reload or the update button
(`clearTileCache`, `RevalidatingFileLoader`). A revisit within `max-age` already sends none. So the requests are a deliberate
freshness check, and the fix keeps it: one version list instead of one round trip per tile (docs/18). HTTP/2 behind a TLS proxy
would also remove the 6-connection limit and shrink this gain; not measured.

Not covered: a revisit after `max-age` expires. The browser revalidates those itself, invisibly to the script, and lowres
then goes through `<img>`, not `fetch`.

## Before and after

`base` = the script before this work; `final` = what ships.

| | base 20,50 | final 20,50 | base 50,10 | final 50,10 |
|---|---:|---:|---:|---:|
| **reload** | 1058 (1051-1070) | **590 (570-594)** | 2273 (2165-2283) | **746 (740-758)** |
| reload, on-screen tiles in | 830 (801-841) | 501 (487-505) | 1669 (1602-2240) | 662 (641-700) |
| first load | 827 (809-833) | 813 (799-842) | 2030 (2021-2047) | 2036 (2021-2045) |
| revisit | 577 (538-599) | 565 (534-593) | 648 (639-650) | 627 (620-646) |
| **pan, new tile p50** | 38 (37-40) | **25 (21-26)** | 87 (86-87) | **26 (24-26)** |
| pan, new tile p95 | 83 (83-84) | 67 (35-69) | 183 (183-185) | 135 (133-151) |
| pan, hires requests | 154 | 170 | 156 | 170 |
| pan, hires KB | 571 | 637 | 580 | 640 |

Empty tiles: 0 in every row and phase. Script: +14 KB in these rows' uncompressed count (wire not measured separately).

- With nothing changed, a reload's ~260 tile revalidations become one list request; no tile goes over the network.
- First load is unchanged: the wait is before the first hires request (at 539/1363 ms, the webapp's JS and `textures.json`), and
  the hires span is only 225/630 ms.
- Fetch-ahead costs 16 unclaimed tiles on this path (+11% hires bytes, 66 KB): `HELD_MAX`, what was still held when the pan
  stopped. Worst case tried, reversing every 2.5 s (`--zigzag`, 3 runs at 20,50): 69 issued, 53 claimed, 16 unclaimed, requests 141 to
  157, p50 still better (50 to 36). The 35% pause did not trigger in these runs, so it has not been exercised.

## Dropped

**On-screen-first** (fetching the camera's tiles ahead of the spiral; removed). 39 of the 42 tiles are on screen in this view
and the spiral already starts at the center, so there is little to reorder.

| | off 20,50 | onscreen 20,50 | off 50,10 | onscreen 50,10 |
|---|---:|---:|---:|---:|
| first load | 821 (814-835) | 796 (781-813) | 2045 (2023-2055) | 2041 (2036-2047) |
| first load, on-screen tiles in | 774 (766-788) | 749 (704-772) | 1872 (1857-1995) | 2001 (1863-2008) |
| reload | 596 (575-614) | 608 (597-610) | 752 (732-780) | **942 (886-953)** |

It gains 3% at 20,50, nothing at 50,10, and hurts reload at 50,10. Only one view was tried; a flatter camera sees fewer of the
square's tiles.

**Paced delivery** (removed after the real-GPU run below). Handing all tiles out a few
per frame delayed everything; letting a frame's first N through at once ("burst") cut that but still costs:

| | off 20,50 | pace 2 | pace 4 | burst 4 | off 50,10 | pace 2 | pace 4 | burst 4 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| reload | 596 | 787 | 621 | 612 | 752 | 950 | 779 | 780 |
| revisit | 567 | 791 | 647 | 613 | 635 | 867 | 710 | 663 |
| pan p50 | 38 | 84 | 68 | 35 (ahead on; final 25) | 87 | 133 | 117 | 35 (ahead on; final 26) |

(`pace` rows come from the run without fetch-ahead, `burst 4` from the final run, whose `final` row has reload 590/746, revisit
565/627.) Frames over 33 ms show no gain: 1 vs 1 here (no draws), and with SwiftShader draws (3 runs, 20,50) reload 54 off vs 56
burst, revisit 58 vs 58. SwiftShader is a weak stand-in for a GPU, so this doesn't settle it: only the real-GPU run can.

## Checks

`tools/check_client_fresh.mjs`, the real webapp against a proxy playing renders and failures: changed tile shown after a reload
with the other 41 not refetched; failing version list; corrupt body; failing fetch-aheads; live `tile` event, including during
that tile's load. No holes in any. The old script fails 5 of them (the new behaviour, including an
event during a load), which shows they can fail.
Gates: `fmt clippy unit arm` pass; `conformance` passes on the testbox (the devbox has no `bluemap/vanilla`). The fixture gates
(render, lowres and storage oracles, acceptance) were not run: nothing they cover changed, and the devbox disk was full.
`check_client_unpack.mjs` 303 tiles identical; `check_client_unpack_browser.mjs` 40 of 40 identical, with and without zstd.
No-script clients and `client-unpack: false`: unchanged code path; `tests/tile_versions.rs` covers the route's 404s.
Browsers without workers or zstd were tested only through the zstd case above; without WebAssembly the script returns before
any of this.

## Real GPU

Intel Arc, canvas 2534x1414, Chrome, 5 interleaved runs per row (30 of 30 valid); median (min-max), ms. Full tables:
[smoothness/load-improvements.md](smoothness/load-improvements.md). All on the optimized map; `base` is the script at `f200a0d`.

| | 20,50 base | final | paced | 50,10 base | final | paced |
|---|---:|---:|---:|---:|---:|---:|
| reload | 1818 (1684-2828) | 1255 (1055-1411) | 1298 (1046-1618) | 3084 (3003-3781) | 1416 (1364-1500) | 1430 (1333-1894) |
| first load | 1695 (1566-2022) | 1905 (1783-2489) | 2043 (1729-2466) | 2680 (2426-3713) | 2537 (2470-2711) | 2666 (2486-3914) |
| pan, new tile p50 | 103 (78-162) | 33 (30-34) | 45 (44-55) | 142 (126-169) | 30 (28-31) | 45 (44-52) |
| pan, new tile p95 | 260 (143-503) | 118 (94-660) | 166 (101-318) | 270 (236-303) | 143 (103-167) | 210 (118-309) |
| reload frames | 106 | 79 | 81 | 190 | 91 | 89 |
| reload frames over 33 ms, % | 3.85 (1.99-4.90) | 3.75 (2.00-5.80) | 2.94 (1.50-3.70) | 1.58 (1.18-2.13) | 3.23 (2.33-3.37) | 3.49 (1.54-6.67) |

- Reload frames over 33 ms, as counts (median frames x median %): 4.1 to 3.0 at 20,50, 3.0 to 2.9 at 50,10: no worse. The percentage
  rises at 50,10 only because a reload now has half the frames. Longest frame is unchanged (about 200 ms).
- Frame times while panning and zooming are unchanged: 25 ms p50/p99 pan, 16.7/25 ms zoom.
- First load: no change expected and none shown; 20,50 reads 12% slower, 50,10 5% faster, ranges overlap, tunnel RTT differed by row.
- Pan 33 and 30 ms is fetch-ahead on a pan in one direction, the best case; with the camera reversing (headless, 20,50) p50 went
  50 to 36.
- Paced delivery removed: its range of frames over 33 ms overlaps `final`, and it adds about 12-15 ms to pan p50.

To repeat, in Git Bash from the checkout, with a devbox build at `~/bmrs-wl40/target/release/bluemap` and the server roots from
`devbox_web_load_dump.sh` under `~/viewtrace-load`. `MSYS_NO_PATHCONV=1` is required, or Git Bash rewrites `--server-bin` and every
run is voided. Use absolute paths; about 40 minutes, keep the Chrome windows visible.

```
git show f200a0d:crates/bm-web/client/unpack.js > "$TEMP/base/unpack-base.js"
scp docs/perf-exp/smoothness_server.sh devbox:smooth/server.sh
W=crates/bm-web/client/unpack.wasm; J=crates/bm-web/client/unpack.js
MSYS_NO_PATHCONV=1 timeout 4h node crates/bm-wasm/bench/smoothness.mjs "C:/Program Files/Google/Chrome/Application/chrome.exe" \
  --out docs/results/smoothness/load-improvements.jsonl --rows base --runs 5 --nets 20,50+50,10 \
  --server-bin /home/agent/bmrs-wl40/target/release/bluemap \
  --variant "base=$TEMP/base/unpack-base.js,$W" --variant "final=$J,$W"
node crates/bm-wasm/bench/smoothness.mjs --report docs/results/smoothness/load-improvements.jsonl
```
