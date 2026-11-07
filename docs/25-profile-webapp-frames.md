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

## Not measured (runs stopped for cost)

`pan`, `zoom`, `flyover`, `idle`, `soak`; compat storage; llvmpipe vs SwiftShader. Static candidates still to
measure, in expected order: lowres PNG decode inside `texImage2D` (main thread, per tile), hires-before-lowres
request order, per-frame CSS2D marker writes, PRBM re-streaming on compat storages, startup texture upload.
