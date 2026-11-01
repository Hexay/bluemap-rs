# DonutSMP ingest throughput: new chunks through `bluemap -u` into optimized storage (2026-11-01)

**Result.** `bluemap -u` sustained **3,400–3,800 new chunks/s (2.1–3.5 full regions/s)** for 10 minutes at 12 render
threads, and **3,170 chunks/s** at 4 threads. A shorter 400 s run held 4,020 chunks/s at 12 threads; 4,560 did not hold.
Output was identical to a clean `-r -f` in every run compared. Estimate for 88 M chunks (300k²): **about 6–7.5 h of
renderer time**, which would keep up with about 180 bots. The renderer is not the bottleneck: the bots are, then the
converter.

## Method

- **Harness:** `tools/ingest_bench.py`, `ingest_model.py`, `ingest_report.py`, `ingest_proc.py`. Raw rows:
  `donutsmp-ingest-2026-11-01.jsonl` (`m12-*` and `mf12` are superseded: earlier boundedness metric, or compare lost to a
  harness bug).
- **Binary:** release glibc build of `3043cc2`. Master's only later commit is a `libz-sys` bump with unchanged zlib
  sources.
- **Machine:** devbox, Ryzen 7 9700X, 8 cores / 16 threads. Each run held `~/bench.lock`. Other work averaged 0.2–0.9
  cores during the runs tabled below, except 5.1 cores on `sc4800`.
- **Writer:** replays real chunks (65,536 core chunks of the 4096² world, 8,889 B each on disk) using the logic of
  `AnvilStore::update`/`flush` read from `acacia-anvil`:
  - 500 ms tick; a column is ready when all 8 neighbours are present or after 10 s;
  - a region is rewritten at most every 10 s, once quiet for 10 s or held for 60 s at most;
  - whole-file rewrite in slot order, temp file + rename; timestamps `max(now, old+1)`.
- **Beyond 64 regions:** chunks repeat by region slot, as `tools/scale_world.py` does.
- **Renderer:** default watcher settings (debounce 5 s, cooldown 60 s, rescan 5 min). World on tmpfs; crawl output on
  disk.
- **Crawl patterns** (bot behaviour is my assumption; no crawl rate was given):
  - *scatter:* N bots at 1.4 chunks/s with view radius 8 and passes 15 apart (`examples/scale/bots.rs`), each mowing a
    128×128-chunk block placed at random. That yields about 19 new chunks/s per bot.
  - *spiral:* one compact strip sped up to the target rate. It is not a physically possible bot; it tests compact
    batching.
- **Bounded:** time-averaged backlog (chunks in written files whose update hasn't finished) in the last quarter of the
  run is at most 1.3× the second quarter, or under 30 s of offered chunks.
- **Latency:** the schedule and task log lines replay the `RenderQueue`. "Visible" means the end of the task that picked
  the chunk up, an upper bound for hires, since tiles become readable as they are appended. Tile figures come from
  final rstate render times (1 s resolution, render start).

## Throughput

| run | threads | window | chunks/s | full regions/s | backlog q2 → q4 (chunks) | cores avg / peak 5 s | RSS avg / peak MB | CPU ms/chunk |
|---|--:|--:|--:|--:|---|---|---|--:|
| scatter, 181 bots | 12 | 600 s | **3,411** | 2.14 | 73k → 70k, bounded | 7.3 / 13.1 | 767 / 1007 | 2.25 |
| spiral | 12 | 600 s | **3,807** | 3.53 | 157k → 125k, bounded | 8.1 / 13.0 | 736 / 981 | 2.32 |
| scatter, 168 bots | 4 | 600 s | **3,167** | 2.01 | 104k → 93k, bounded | 3.8 / 5.3 | 383 / 1187 | 1.28 |
| scatter, large map (k=8), 143 bots | 12 | 600 s | 2,698 | 1.71 | 52k → 39k, bounded | 6.1 / 13.2 | 700 / 1017 | 2.26 |
| scatter, 210 bots | 12 | 400 s | 4,020 | 2.10 | 130k → 124k, bounded | 8.1 / 13.2 | 777 / 1049 | 2.02 |
| scatter, 238 bots | 12 | 400 s | 4,556 | 2.37 | 168k → 316k, **grows** | 8.2 / 12.8 | 799 / 1039 | 1.80 |
| spiral | 12 | 300 s | 4,809 | 4.32 | 111k → 429k, **grows** | 9.2 / 12.4 (others 5.1) | | |
| scatter, 168 bots | 4 | 300 s | 3,288 | 1.03 | 75k → 89k, bounded | 3.5 / 5.3 | | |
| scatter, 196 bots | 4 | 300 s | 3,834 | 1.17 | 101k → 233k, **grows** | 3.8 / 5.3 | | |
| one real-speed bot (spiral, 1.4 chunks/s) | 12 | 300 s | 22.8 | 0.013 | 104 → 141 | 0.05 / 0.33 | 162 / 597 | 2.44 |

CPU ms/chunk covers crawl plus drain where the run settled (600 s runs except large, and the real-speed bot); otherwise
the whole run up to stop, which understates cost because the backlog is left unrendered.

A 300 s spiral at 3,809 chunks/s (`schk`) tripped the bounded rule (45k → 160k) because its second quarter is still
ramping up. The 600 s run at the same rate is bounded, and I took that as the verdict.

**Limits of the sweep:**
- 10-minute runs top out near 3,800 chunks/s: world bytes live on a 32 GB tmpfs. 4,020 chunks/s therefore has only 400 s
  behind it.
- Rungs are not evenly spaced; the 12-thread limit is bracketed at 4,020–4,556 chunks/s, the 4-thread limit at
  3,288–3,834.
- 4-thread spiral was skipped to stay near the time budget.

## Latency (seconds, p50 / p99)

| run | store debounce | watcher debounce | queue wait | task | **offered → visible** | last chunk under tile → final render start | last write → final render start |
|---|---|---|---|---|---|---|---|
| scatter, 12 t, 3,411/s | 24.0 / 58.7 | 5.0 / 54.5 | 1.5 / 11.8 | 10.9 / 20.6 | **44 / 103** | 38 / 225 | 13 / 61 |
| spiral, 12 t, 3,807/s | 10.0 / 27.1 | 29.5 / 54.5 | 1.1 / 8.7 | 2.9 / 13.3 | **48 / 77** | 49 / 80 | 37 / 63 |
| scatter, 4 t, 3,167/s | 24.0 / 58.6 | 5.0 / 55.0 | 3.9 / 19.3 | 17.6 / 25.5 | **54 / 112** | 43 / 215 | 18 / 61 |
| real-speed bot | 22.0 / 51.0 | 5.0 / 55.2 | 0.0 / 0.2 | 0.2 / 0.3 | **29 / 78** | 30 / 202 | 5 / 55 |

Debounce dominates at every rate. The tile p99 near 200 s is real but separate from rendering: a tile needs all 9 chunks
under it, so its last chunk can wait for the bot's next pass, and a later re-render also moves the "last render" time.

## Amplification and disk

| run | tiles rendered ÷ final | skip attempts | region rewrites per region | `.mca` written ÷ final (KB/chunk) | renderer written ÷ final (B/chunk) | lowres saves |
|---|--:|--:|--:|---|---|--:|
| scatter, 12 t | 1.058 | 452k | 2.27 | 2.11× (18.7 / 8.9) | 1.29× (2,464 / 1,914) | 25,422 |
| spiral, 12 t | 1.087 | 388k | 2.63 | 1.87× (16.7 / 8.9) | 1.17× (2,393 / 2,042) | 11,945 |
| scatter, 4 t | 1.065 | 417k | 2.27 | 2.11× (18.7 / 8.9) | 1.29× (2,467 / 1,914) | 23,819 |

- Boundary re-renders are small because a tile with any of its 9 chunks missing is not rendered at all
  (`failed_precondition`: `not-generated`). The cost shows up instead as skip attempts, about as many as tiles rendered
  in scatter. Each one clears 1024 lowres columns but renders nothing.
- **Understated:** a relit chunk here has the same bytes and a new timestamp, so its re-rendered tile comes out unchanged
  and nothing is appended (bundle file bytes = live bytes in all compared runs). With real relighting those tiles differ
  and append. Worst case that adds the 6–9% re-render share to hires bytes.
- **Conservative on the other side:** every relit column counts as changed, where the real store keeps the timestamp if
  the NBT is identical.

## Correctness

Crawl output against a clean `-r -f` of the same world: per-tile blob digests from the bundles, lowres file digests, and
tile states.

| run | hires tiles | lowres files | result |
|---|--:|--:|---|
| scatter, 12 t, 300 s (3,536/s) | 246,623 | 3,910 | **identical** |
| spiral, 12 t, 300 s (3,809/s) | 284,248 | 1,375 | **identical** |
| scatter, 4 t, 600 s (3,167/s) | 451,386 | 5,492* | **identical** |
| real-speed bot, twice | 1,654 | 44 | **identical** |

\* From the clean run's log; the file-by-file compare of this run reported no missing, extra or differing lowres file.

No holes and no wrong tiles. The 12-thread 600 s runs were not compared: scatter lost its compare to a harness bug and
spiral didn't fit in tmpfs, hence the 300 s checks at similar rates. The large-map run wasn't compared either; its
hires fixture was left unmodified (0 files newer).

I used my own comparator instead of `compare-webroots`, because a compat conversion wouldn't fit on disk. Negative control
on existing fixtures: 12- vs 16-thread renders compare identical; the edited-world render reports 39 hires and 14 lowres
differences. Bundle parser: 1,046,529 tiles at k=8, equal to rstate's `rendered` count.

**Limit of this check:** the world's bytes are vanilla chunks, so it tests update ordering and neighbour handling, not
`acacia-anvil`'s conversion or light.

## Findings

1. **Map size doesn't matter.** Over the k=8 map, 2.26 ms CPU per chunk against 2.25 on an empty one. Region updates touch
   only their own cells. Startup to "up-to-date" took 0.6 s at k=8, but that run's log is deleted, so I didn't confirm how
   many regions that pass covered; docs/22 gives about 4 min CPU at k=73 for the same no-op pass on the slower testbox.
2. **4 threads get about 93% of 12.** 3,167 vs 3,411 chunks/s, at 1.28 vs 2.25 ms CPU per chunk. The clean render shows
   the same curve: 1.03 ms/chunk at 4 threads, 1.97 at 12, wall rate 4,130 vs 5,380 chunks/s. So it isn't specific to
   ingest. This CPU has 8 physical cores, so SMT sharing is the likely cause; I didn't test that. `render-thread-count: 4`
   looks like a good setting for a shared host.
3. **The saturated 12-thread run averages only about 8 of 12 threads, cause not established.** Ruled out:
   - disk: tmpfs output gives 7.9 cores against 7.75, and the same backlog growth;
   - the persist thread: about 0.34 core (`diag12`, 82 CPU-s in 240 s). PNG deflate is about 7–8% of cycles.

   Not tested: only 2 regions are in flight at once (`update.rs`), which could idle workers on partial regions, and SMT
   saturation. Ingest reaches about 75% of clean-render rate at the limit (4,020–4,556 vs 5,380–5,990 chunks/s at 12
   threads).
4. **The 5-minute rescan re-schedules regions already rendered, a no-op (product behaviour, harmless).**
   - `Fingerprints` in `watch/files.rs` only refreshes on rescan, so every file changed since the last rescan looks
     changed, even though the inotify path already handled it.
   - Rerun with the log kept: start 19:57:43, rescan 20:02:43, burst of schedules at 20:02:48 (+5 s debounce), and 8 tasks
     up to 20:03:42 with `0 tiles rendered, 0 skipped`, some delayed by cooldown.
   - Those are what the `bot1` log showed at 19:45:26. The bot's real tasks did render: 1,790 tiles rendered, 1,654 final,
     output identical to a clean render. This is not a harness bug and not lost tiles.
   - Cost: scatter 12 t had 8,645 schedules for 5,816 file versions. Each is a header compare only. The re-arm can push a
     pending real update out by up to the cooldown. Not fixed, as instructed.
5. **Disk-full check.** No row needs a rerun on that account. All tabled rows have 0 `WARNING`/`ERROR` lines and none
   aborted. Each crawl tick checked free disk (3 GB guard at the time). Logs of finished runs were deleted, so this rests
   on those recorded counts plus the identical compares. The guard is now 10 GB, and runs refuse to start below it.

## Estimate for 300k² (88 M chunks): extrapolated, not measured

| | at 3,411/s (12 t, checked) | at 4,020/s (12 t, 400 s) | at 3,167/s (4 t, checked) |
|---|--:|--:|--:|
| renderer wall time | **7.2 h** | **6.1 h** | **7.7 h** |
| bots kept up with, at about 19 chunks/s | ~180 | ~210 | ~165 |

- CPU about 55 core-hours at 12 threads, about 31 at 4.
- Optimized output about 170 GB final (1,914–2,042 B/chunk), about 220 GB written. Converter writes about 2× its roughly
  710–780 GB final world.
- Assumes chunks of about 8 KB (8.9 KB here; zlib cost scales with bytes but read and parse are about 2% of render CPU).
- **Real DonutSMP terrain could cost more.** This is one vanilla seed, tiled, with no builds. Render CPU follows faces per
  tile, not chunk bytes, so dense player bases will be slower. It also isn't converter output.

## Bottleneck

**Bots first, then the converter; the renderer has headroom over both.**

| stage | rate | source |
|---|--:|---|
| 8 bots at `examples/scale` speed | ~150–190 chunks/s, so 88 M takes about 5.5–7 days | that harness's model |
| converter (`acacia-anvil` store, one writer) | 1,179–1,402 columns/s (testbox, 12 t), 2,155 (22 t) | its README, 2026-10-24, synthetic terrain; **not re-measured** |
| renderer | 3,167–4,020 chunks/s | this document |

- At 8 bots the renderer idles at under 5% of capacity. Latency is then about 30 s p50 and 80 s p99, nearly all debounce.
- With enough bots the single store limits first, at about 60–115 bots: 88 M in about 11–21 h.
- The converter and renderer compete for cores on one host; this benchmark doesn't include converter CPU.

## Reproduce

```
MSYS_NO_PATHCONV=1 BM_TESTBOX=devbox py -3 tools/testbox.py run ingest --sync --detach --job j -- \
  'python3 tools/ingest_bench.py run NAME --pattern scatter --rate 3800 --duration 600 --settle --compare'
```

Add `--threads 4`, `--pattern spiral`, `--large`, `--speed 1.4` (real-speed bot), `--perf 45` (profile), or
`sweep NAME --pattern P --start R`. Binary: `~/ingest/bin/bluemap`. Results append to `~/ingest/results.jsonl`.
