# 25 — Profile: webapp frames, memory and tile loading

Scope: what the upstream 5.29 webapp costs a viewer per frame and per view, and what our client script can fix
without changing the bundle (route A). Measured 2026-11-03 on the devbox (no GPU: headless Chrome 1248, SwiftShader;
load average 40–60 from other sessions). Frame rates here are software-rendered; rank by the GPU-independent
numbers (main-thread ms per frame, draw calls, triangles, tile latency, view load time, memory).

## Harness (`tools/webperf/`)

```
node tools/webperf/run.mjs <map url> --out r.jsonl --label base [--inject patch.js]   # 1 initial-load run, 960×600
node tools/webperf/compare.mjs r.jsonl … --markdown       # medians per label/scenario(/contended)
```

- `probe.js` wraps the webapp's render loop and tile managers; pasted into a real browser's console it gives true-GPU
  numbers (`bmPerf.start()`, move, `bmPerf.stop(); bmPerf.report()`).
- Scenarios: `initial-load` (default), `pan`, `zoom`, `flyover`, `idle`, `soak`; `--inject` overlays a candidate fix.
  The tables below used `--window 1280,800 --runs 3`; pass them to compare with them.
- Box limits (`box.mjs`; earlier uncapped runs loaded the shared devbox for a long time): each Chrome runs in its own
  `systemd-run --user --scope` unit `bm-webperf-<pid>-<n>`: `CPUQuota` = `--cpus` (2) × 100 %, `MemoryMax` =
  `--max-rss` (2500 MB), no swap, `TasksMax=1024`, nice 10, `RuntimeMaxSec` = run timeout + 30 s (kills it even if
  node is SIGKILLed); teardown stops the unit, also on Ctrl-C, SIGTERM and crashes; stale units of dead runs are
  stopped at start. Chrome gets no session D-Bus: with one its browser process moves itself into its own
  `app-org.chromium.Chromium-<pid>.scope`, out of the limits. Without `systemd-run --user`: plain spawn, RSS watch, warning.
- Hard limits: `--run-timeout` (180 s) per run, `--max-minutes` (15) per invocation; on expiry Chrome is stopped,
  completed runs are summarized and the aborted one is appended to `--out` as `aborted`; exit 2 / 3 (4: preflight).
- Preflight: no run starts unless MemAvailable > `--max-rss` + 1 GB. Each result's `box` holds load1, cpu PSI some
  avg10 and MemAvailable before/after, the scope's CPU and throttled ms and memory peak; `contended` when PSI passed
  `--contended-psi` (40). `--wait-load <psi>` waits (5 s polls, `--wait-max` 10 min) for PSI to drop first.
- One capped Chrome per box: a run waits (up to `--wait-max`) while another live run.mjs's `bm-webperf-*` scope exists.
- `--trace` is the expensive option (an earlier traced run crashed Chrome); leave it off unless a question needs it.

## Initial load (4096² real world, optimized storage, 1280×800, median of 3; raw: `docs/results/25-webapp/`)

| | stock | tile slots filled | stock, 640×400 |
|---|---:|---:|---:|
| view complete | **13.8 s** | **4.7 s** | 5.6 s |
| tiles in flight, mean (limit 8) | 3.3 | 7.9 | 4.4 |
| tile starts per rendered frame | 1.85 | 3.18 | 2.32 |
| lowres tile latency p50 / p95 | 261 / 453 ms | 239 / 317 ms | 130 / 303 ms |
| longest task | 440 ms | 183 ms | 178 ms |
| tasks > 50 ms | 111 | 53 | 74 |
| main ms per frame p50 / p95 | 1.8 / 12.3 | 1.3 / 27 | 1.6 / 18.8 |
| terrain height ms per frame p95 | 0.2 | 0.2 | 0.2 |
| draw calls / triangles p50 | 50 / 198k | 56 / 228k | 55 / 223k |
| fetched | 216 tiles, 9.9 MB | same | same |
| JS heap peak | 23 MB | 24 MB | 16 MB |

- The default view opens at distance 1500: lowres only, no hires tile is requested.
- **Tile starts are paced by the frame rate.** Upstream `TileManager.loadCloseTiles` starts one tile and re-arms with
  `setTimeout(0)`; with long frames about one such timer runs per frame, so the 8 slots are rarely full. Starting
  tiles until the slots are full (`client/slots.js`, on by default, `{"slots":0}` turns it off) cut the view from
  13.8 to 4.7 s with the same bytes. The smaller window (cheaper frames, no fix) loading 2.5× faster points the same
  way. On a 60 fps GPU the gain shrinks; slow laptops and phones see most of it. Server time per tile: 1–3 ms.
- Terrain-height raycast (`MapHeightControls`, every frame) is 0.2 ms p95, below the static estimate; low priority.
- 320×200 runs recorded no frames (harness issue) and are left out.

## Many shape markers (issue #904; 2026-11-07)

9,300 generated claims in one set (`tools/gen_markers.py --radius 2048`, 4.7 MB) on the same world, served with
`bluemap -w`; client script with `{"markerBatch":0,"markerPoll":0}` (upstream behaviour) vs defaults (docs/18
"Many shape markers"). One run each, 960×600, capped Chrome, every run contended (PSI 59–72); raw:
`docs/results/25-webapp/markers-9300-ab.jsonl`.

| | off: load | off: idle | on: load | on: idle |
|---|---:|---:|---:|---:|
| draw calls per frame p50 | 10,475 | 10,503 | 49 | 77 |
| main ms per frame p50 / p95 | 292 / 587 | 345 / 429 | 59 / 163 | 92 / 176 |
| main busy ms/s | 200 | 202 | 91 | 45 |
| geometries alive | 10,440 | 10,440 | 14 | 14 |
| triangles k p50 | 278 | 418 | 384 | 524 |
| JS heap peak MB | 188 | 194 | 149 | 126 |
| Chrome peak MB | 1,021 | 1,006 | 707 | 709 |
| view complete s | 36.6 | 30.8 | 28.9 | 26.5 |

- Draw calls −99%, main-thread time per frame −70–80%. Triangles rise ~100k: a merged batch is drawn whole where
  upstream frustum-culls per marker; trivial for a GPU, visible here only because SwiftShader rasterises on the CPU.
- Look and clicks: `node tools/webperf/marker_check.mjs <map url> <out dir>` (capped Chrome per variant): overview and
  close-up screenshots match upstream apart from a 1–2 px camera offset (depth-tested outlines hidden by terrain the
  same way); clicks go through `mapViewer.handleMapInteraction` (Hammer's tap needs press→release within 250 ms,
  which a 1 fps software page misses) and gave the same popup per marker in both variants (3/5 right, the same two
  misses: upstream hit-testing). Not checked: real-GPU fps.

## Not measured (runs stopped for cost)

`pan`, `zoom`, `flyover`, `idle`, `soak`; compat storage; llvmpipe vs SwiftShader. Static candidates still to
measure, in expected order: lowres PNG decode inside `texImage2D` (main thread, per tile), hires-before-lowres
request order, per-frame CSS2D marker writes, PRBM re-streaming on compat storages, startup texture upload.
