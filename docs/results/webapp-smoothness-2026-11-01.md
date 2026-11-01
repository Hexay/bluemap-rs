# Webapp smoothness on a real GPU: Java BlueMap vs bluemap-rs (2026-11-01)

**Answer: no measurable frame-pacing difference during pan, seen-tile pan or zoom.** At this resolution every row sits at
the same ~25 ms frame time, so these runs cannot show a difference smaller than that limit. What ours does improve is how
soon things arrive: with optimized + script, hires tiles appear 2–3.6× sooner during a pan and first load is shorter.
That is less pop-in, not steadier frames.

Every metric per phase and link: [`smoothness/report.md`](smoothness/report.md). Raw rows: `smoothness/rows.jsonl`.

## Method

- Client: headed Chrome, fresh profile per run, Intel Arc (`ANGLE (Intel, Intel(R) Arc(TM) Graphics, Direct3D11)`), 120 Hz
  panel (measured p50 8.3–8.4 ms), dpr 2, canvas 2534×1414. No software renderer in any run.
- Servers on the devbox under `~/bench.lock`, through `ssh -L`, restarted per row-run and warmed 3 s on tiles far from the
  path. `~/viewtrace-load`: **j529** Java 5.28 CLI serving Java's render with 5.29 webapp files, **rc** ours (0.3.1, by
  `--version`) on the same render, **ro** ours optimized + script, **rn** the same without the script.
- Links by CDP throttling: 20 ms/50 Mbit/s and 50 ms/10 Mbit/s. The tunnel adds to them: ping 11 ms, HTTP round trip
  median 12–16 ms on ours (28–29 ms on Java, which includes its response time). Raw ssh throughput was ~51 Mbit/s, so "50 Mbit/s"
  sits at the tunnel's own limit.
- Phases: (a) first load until hires tiles are in, (b) 5 s idle, (c) pan 720 blocks in 12 s, (0,0)→(360,0)→(360,360), via
  `controlsManager.position`, following the wall clock, (d) the same pan again, (e) zoom 60→1500→60 in 8 s, (f) reload.
- Runs: 38 valid of 39, interleaved. 5 per row-link, except ro and rn at 50,10 with 4. Ranges below are min–max across runs.
- Local machine: outside CPU load at most 1.9% of capacity, at least 5.6 GB free, on mains. One run voided (window hidden).

## Not run or not measured

- `--uncapped` (`--disable-gpu-vsync --disable-frame-rate-limit`), `--scale 1` and the unthrottled link: stopped to end
  the disruption on the user's screen. So nothing here shows whether the 25 ms limit hides a difference.
- The last two runs (ro and rn at 50,10, run 5). A covered window hung the campaign for 17 minutes: rAF stopped and the pan
  had no timeout. The hung run left no row. Pan and zoom now have a deadline that voids the run; **that fix has only been
  syntax-checked, not run.**
- The cause of the 25 ms limit. Main-thread evidence points away from the page (no task over 50 ms, about a third of the
  thread busy), so the GPU or compositor at 2534×1414 is likely, but no GPU trace of a pan was taken.

## Is it smoother? Per phase

| phase | answer | evidence |
|---|---|---|
| (a) first load | Shorter, not smoother | Same number of bad frames in less time: ~4 frames over 50 ms in every row. Percentages look worse for ro only because it has fewer frames. Longest task no better |
| (b) idle | No claim | Not truly idle: 0.6–7.4 MB of lowres still arrives during it. Webapp redraws at 20 Hz when idle while rAF runs at 120 Hz |
| (c) pan, new tiles | Frames the same; tiles sooner with ro | p50, p95, p99 identical. Tile table below |
| (d) pan, seen tiles | Frames the same, as expected | No network (0–8 KB). ro's tile p95 is lower than Java's |
| (e) zoom | Same | p50 16.7, p99 25.1 in every row, no frames over 50 ms |
| (f) reload | Much shorter; no smoother, possibly worse at 20,50 | Java re-downloads ~11 MB, ours 20 KB. At 20,50 ours has more frames over 33 ms (table below) |

## Frame times (ms, median of runs)

Pan, seen-tile pan and zoom. Ranges are in `report.md`; p50/p95/p99 vary by at most 0.1 ms except zoom p99 25.6.

| link | row | (c) p50 / p99 / max | (c) >33 % | (d) p50 / p99 / max | (d) >33 % | (e) p50 / p99 / max | longest task c, d, e |
|---|---|---:|---:|---:|---:|---:|---:|
| 20,50 | j529 | 25.0 / 25.1 / 25 | 0.00 | 25.0 / 25.1 / 33 | 0.18 | 16.7 / 25.1 / 25 | none over 50 |
| 20,50 | rc | 25.0 / 25.1 / 25 | 0.00 | 25.0 / 25.1 / 25 | 0.00 | 16.7 / 25.1 / 25 | none over 50 |
| 20,50 | ro | 24.9 / 25.1 / 25 | 0.00 | 25.0 / 25.1 / 33 | 0.18 | 16.7 / 25.1 / 25 | none over 50 |
| 20,50 | rn | 24.9 / 25.1 / 25 | 0.00 | 25.0 / 25.1 / 33 | 0.18 | 16.7 / 25.1 / 25 | none over 50 |
| 50,10 | j529 | 24.9 / 25.1 / 25 | 0.00 | 25.0 / 25.1 / 33 | 0.18 | 16.7 / 25.1 / 25 | none over 50 |
| 50,10 | rc | 24.9 / 25.1 / 25 | 0.00 | 25.0 / 25.1 / 33 | 0.18 | 16.7 / 25.1 / 25 | none over 50 |
| 50,10 | ro | 25.0 / 25.1 / 25 | 0.00 | 25.0 / 25.1 / 25 | 0.00 | 16.7 / 25.1 / 25 | none over 50 |
| 50,10 | rn | 24.9 / 25.1 / 25 | 0.00 | 25.0 / 25.1 / 42 | 0.36 | 16.7 / 25.1 / 25 | none over 50 |

Frame intervals land on multiples of the 8.33 ms vsync, and 16.7 is exactly two of them, so the `>16.7 %` column in
`report.md` is partly timing jitter. Use `>33` and `>50`.

## Hires tiles during a pan (entered the manager's square → loaded, ms)

~156 tiles per pan in every row. Another 12 entered the square but never loaded, in every row (the manager's spiral stops
short of some corners). The manager's square moves in 50-block steps, so this starts at the step, not the exact crossing.

| link | row | pan KB | (c) p50 | (c) p95 | (c) max | (d) p50 | (d) p95 |
|---|---|---:|---:|---:|---:|---:|---:|
| 20,50 | j529 | 9655 | 197 (173–227) | 401 (337–619) | 522 (383–1003) | 54 (34–83) | 112 (92–167) |
| 20,50 | rc | 9487 | 204 (195–213) | 509 (351–675) | 710 (383–936) | 78 (46–93) | 165 (92–192) |
| 20,50 | **ro** | **580** | **95 (88–102)** | **183 (169–251)** | **219 (200–300)** | 43 (29–53) | 70 (54–83) |
| 20,50 | rn | 9050 | 233 (183–245) | 434 (314–700) | 826 (376–1101) | 80 (59–93) | 170 (96–213) |
| 50,10 | j529 | 9655 | 429 (422–517) | 860 (794–916) | 1042 (867–1176) | 59 (48–95) | 136 (96–238) |
| 50,10 | rc | 9598 | 433 (421–566) | 793 (780–1050) | 868 (859–1368) | 51 (47–80) | 97 (94–167) |
| 50,10 | **ro** | **580** | **119 (112–135)** | **243 (227–268)** | **268 (245–319)** | 35 (33–46) | 55 (44–80) |
| 50,10 | rn | 9050 | 424 (410–440) | 768 (743–779) | 818 (801–990) | 76 (54–99) | 148 (112–171) |

rc and rn match Java within noise, so the gain comes from the optimized format with the script, not from our server alone.
ro's p50 ranges do not overlap Java's in (c). In (d), j529 and rc read the same cached bytes yet differ by up to 24 ms at
p50: treat that as the noise floor.

## Load and reload

Counts are frames per run, median (range): percentages are not comparable because the phases differ in length.

| link | row | first load ms | reload ms | load frames >50 | reload frames >33 | reload frames >50 | longest task load | longest task reload |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 20,50 | j529 | 2175 (1929–2438) | 3580 (3354–6344) | 4 (2–5) | 2 (1–3) | 1 (1–1) | 254 (153–375) | 330 (234–446) |
| 20,50 | rc | 1907 (1707–2144) | 1840 (1563–2068) | 3 (3–4) | 11 (3–11) | 3 (3–4) | 247 (173–273) | 328 (237–498) |
| 20,50 | ro | 1672 (1555–1810) | 1838 (1661–2057) | 4 (3–5) | 5 (2–9) | 1 (1–2) | 251 (208–316) | 405 (242–489) |
| 20,50 | rn | 2072 (1901–2951) | 2054 (1700–2286) | 4 (4–4) | 8 (4–10) | 4 (2–5) | 211 (176–479) | 373 (291–507) |
| 50,10 | j529 | 5071 (4995–5168) | 10762 (9940–11262) | 4 (4–4) | 5 (4–7) | 1 (1–3) | 262 (191–341) | 378 (320–405) |
| 50,10 | rc | 4389 (4265–4427) | 3151 (3072–3372) | 4 (3–4) | 4 (3–6) | 2 (1–2) | 263 (227–336) | 435 (383–479) |
| 50,10 | ro | 2917 (2706–3138) | 3184 (3079–3198) | 4 (4–4) | 5 (3–6) | 1 (1–1) | 413 (229–535) | 366 (322–452) |
| 50,10 | rn | 4621 (4331–4715) | 3297 (3150–3376) | 4 (4–4) | 4 (3–4) | 2 (2–2) | 348 (267–380) | 377 (290–526) |

- **Where ours is worse:** reload at 20,50 has more frames over 33 ms in all three of our rows (median 5–11 vs 2), with
  overlapping ranges for ro. The same work seems to land in about half the time, but that explanation is untested.
- Reload is fast mainly because ours sends validators on map data: 20 KB vs ~11 MB (see
  `devbox-webapp-load-breakdown-2026-11-01.md`).
- Longest task is the webapp's own start-up in every row. ro's 413 ms median at 50,10 has a range overlapping Java's.

## CPU and memory (pan, median of runs)

| link | row | main busy ms (c) | main busy ms (d) | renderer MB (c) | GPU process MB (c) | workers MB (c) | JS heap MB (c) |
|---|---|---:|---:|---:|---:|---:|---:|
| 20,50 | j529 | 3964 (3167–4596) | 3184 (2685–3902) | 666 | 774 | - | 18 |
| 20,50 | rc | 4699 (3287–4742) | 4167 (2860–4314) | 677 | 761 | - | 15 |
| 20,50 | ro | 3540 (2706–3783) | 3077 (2373–3274) | 497 | 783 | 11 | 15 |
| 20,50 | rn | 4113 (3802–4871) | 3810 (3065–4956) | 669 | 798 | - | 16 |
| 50,10 | j529 | 4000 (3650–5238) | 3415 (2972–4617) | 652 | 752 | - | 17 |
| 50,10 | rc | 3933 (3769–4305) | 3088 (2987–4083) | 672 | 757 | - | 15 |
| 50,10 | ro | 3059 (2911–3476) | 2558 (2484–3056) | 498 | 777 | 11 | 18 |
| 50,10 | rn | 4162 (3635–4666) | 3787 (3092–4159) | 666 | 780 | - | 12 |

- Main busy time covers the pan plus its settling, up to 0.8 s longer for rows other than ro. ro's main thread is less
  busy, clearly so only at 50,10 in (c). None of it reached frame times on this CPU.
- ro's renderer holds ~170 MB less at the end of a pan. The cause was not investigated.
- Memory is working set at phase end, not peak. Worker MB is V8 heap plus backing stores from `Runtime.getHeapUsage`.

## What one Intel Arc machine can't tell you

- No phones, no discrete GPU, no weak CPU. ro's lower main-thread load might show up as fewer dropped frames on a slower
  CPU, or the limit might lift on a stronger GPU and expose differences. Neither was tested.
- rAF intervals show when the page got frames, not when they reached the screen.
- Only fully covered windows are detected (`document.hidden`); partial covering is not. Focus was held at every phase end.
- One world, one path, one speed (60 blocks/s), default view distances.
- j529's server is Java 5.28 serving 5.29 webapp files, as in earlier results; no 5.29 server was measured.

## Reproduce

First `scp docs/perf-exp/smoothness_server.sh devbox:smooth/server.sh`. The roots are `~/viewtrace-load/{j529,rc,ro}`.

```
node crates/bm-wasm/bench/smoothness.mjs "C:/Program Files/Google/Chrome/Application/chrome.exe" --out docs/results/smoothness/rows.jsonl
node crates/bm-wasm/bench/smoothness.mjs --report docs/results/smoothness/rows.jsonl
```

Resumes from `--out`, about 75 s per run. The side runs that could expose the limit (use a separate `--out`):
`--rows j529,ro --nets 20,50 --runs 3 --uncapped`, and the same with `--scale 1`.
