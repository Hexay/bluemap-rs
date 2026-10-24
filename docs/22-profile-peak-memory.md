# 22 — Peak memory vs map extent (target: 300k × 300k, `format: optimized`)

Question: what does a forced render hold in memory as the map grows, and does it fit at DonutSMP scale
(300k² blocks ≈ 341k regions, ≈ 88 M hires tiles)?
Answer: before this work, peak RSS grew by ~97 MB per 4096 blocks of map **width** (open lowres tiles) plus an
O(area) render-state term, so a 300k² render would have needed **≈ 9.7 GB**. Lowres tiles now spill to storage past
a 128 MB budget, and render-state cells leave memory once no remaining region touches them: **≈ 1.1 GB**
extrapolated, flat from k=2 on (measured 986 → 1006 MB from k=2 to k=4). Outputs are byte-identical.

Setup: testbox (12 cores, 31 GiB, HDD, shared and often at load 40–100), glibc build (the plugin's musl+mimalloc
build differs in allocator slack, not in what is allocated). World: the 4096² real world (docs/14) tiled k×k by
hardlinks (`tools/scale_world.py`), 12 render threads, forced `-r -f` into optimized file storage. Peak RSS is
`/usr/bin/time` max RSS; heap attribution is heaptrack (`heaptrack_print -p -m 0`, aggregated by the innermost
bluemap-rs source lines).

## Scaling, before

| k | map | regions | peak RSS (ladder, master) | open lowres tiles at the front (simulated) |
|---|---|---|---|---|
| 1 | 4096² | 64 | 858 MB | 48 (96 MB) |
| 2 | 8192² | 256 | 970 MB | 95 (190 MB) |
| 4 | 16384² | 1024 | 1161 MB | 189 (378 MB) |
| 8 | 32768² | 4096 | 1489 MB | 380 (760 MB) |
| 73 | 299008² | 341 k | — | 3546 (7.1 GB) |

- **Attribution** (heaptrack heap peak): k=1 627 MB, k=4 793 MB. Of the +166 MB, lowres tiles are +159 MB
  (`LowresTile::new` from `LowresTileManager::write`: 76 → 235 MB). Nothing else grows. The constant part is
  per render worker: PRBM buffers 107–118 MB (`prbm.rs:55` from `render.rs:92`), mesh buffers ~250 MB
  (`mesh.rs:52–59, 108`), chunk data ~40–70 MB, the compact encoder ~60 MB (owned by the CPU/µarch agents). RSS runs
  ~290 MB above the heap peak (glibc arenas).
- **Why width.** Regions run in Java's order: by distance from region 0,0, a spiral. A lowres tile stays in memory
  until every region that can write to it is done, so the open tiles are the ring between finished and unfinished
  regions: ~48·k tiles × 2 MB (501×501 px × 2 × u32). Any order has such a boundary at some point (it is as long
  as the map is wide), so changing the order does not remove the term; only spilling does.
- **Area term.** `CellStore` never dropped a cell: every tile-state cell (32×32 tiles, ~5 KB) and chunk-hash cell
  (128×128 chunks, 64 KB) of the map stayed loaded until the update ended. That is 0.34 MB·k²: invisible at k≤8
  (≤ 22 MB), **1.83 GB at k=73** (86 k tile cells + 22 k chunk cells). The same held for a no-op `-r` and the
  startup pass of `-u`.

Fit (MB): `peak ≈ 760 + 97·k + 0.34·k²` (k=8: 1558 predicted, 1489 measured). At k=73: 760 + 7092 + 1830 + ~20
(plan maps) ≈ **9.7 GB**.

## Fixes

1. **Lowres spill budget** (`bm_map::lowres`, `LowresTileManager::with_budget`; budget 128 MB in
   `bm-engine/src/persist.rs`). When a new dirty tile would exceed the budget, the least recently written one is
   saved to the store **without cascading** and dropped. Its next write, or the flush that finalises it, loads it
   back. Cascades still run once per final tile, so the next LOD gets the same pixels, and the PNG round trip is
   lossless (8-bit RGBA, colour and metadata halves). A spilled tile that turns out unchanged is not rewritten.
   Final flushes reload spilled tiles in batches of half the budget.
2. **Render-state release** (`Plan` counts pending regions per tile-state and chunk-hash cell, as it already did per
   lowres tile; `CellStore::release` writes a finished cell and drops it). A cell is finished when no remaining
   region's tiles or chunk hashes fall in it, so it is never reloaded in the same update. That matters because
   writes go through the persistence queue. Region-state cells stay loaded (64×64 regions per cell: 1.4 MB at
   300k²).
3. `CellStore` hashes cell positions with FxHash: SipHash was 15% of a no-op `-r` (`chunks_changed` looks a chunk
   cell up per chunk per tile).

## Results (locked A/B, same box, interleaved)

| k | peak RSS base → new | CPU s (user+sys) base → new | lowres saves base → new | output |
|---|---|---|---|---|
| 2 | 986 → 986 MB | 1125 → 1123 | 344 → 459 | identical |
| 4 | 1200 → 1006 MB (−16%) | 4513 → 4588 (+1.7%, box noise ±5%) | 1224 → 3389 | identical (1585 files) |

- "identical": every lowres PNG, JSON and decoded rstate cell (render timestamps masked) byte-equal; hires is not
  touched by these changes. Gates: `cargo test -p bm-map -- --ignored` (lowres goldens also with a 4-tile budget),
  `cargo test -p bm-storage --release --test oracle_optimized -- --ignored`, `tools/accept.py` (incl. incremental,
  `-u` watch, drop-in) and `tools/accept_optimized.py`: all pass.
- At k=2 the RSS peak is not set by the lowres front (95 tiles, budget 63), so it does not move. At k=4 the front
  is 189 tiles; the 250 MB it held above the budget is 194 MB of RSS.
- **Spill cost.** 1.8 extra PNG saves per lowres tile at k=4, on the persistence thread (4 encode threads at
  flushes). At k=73 the LOD-1 ring is 2386 tiles against 63 in budget, so expect about one spill per LOD-1 tile plus
  ~5 per LOD-2 and ~24 per LOD-3 tile (they receive cascades over many revolutions): ≈ 0.45 M spills × ~25 ms
  (encode + reload) ≈ 3 CPU-hours over a ~430 CPU-hour render (<1%). It runs on the persistence thread, which is
  otherwise mostly idle.

Extrapolated peak at 300k² (k=73), new: 1006 MB (k=4 plateau) + open rstate cells (1755 × 5 KB + 748 × 64 KB ≈
57 MB) + plan maps (~20 MB) ≈ **1.1 GB**, against ≈ 9.7 GB before.

## What still grows with the map (all small)

| structure | where | size at 300k² | plan |
|---|---|---|---|
| plan: region order + index, pending counts (lowres, tile/chunk cells) | `bm-engine/src/plan.rs` | ~20 MB | fine; a region bitmap would halve it |
| region-state cells | `MapRegionState` | 1.4 MB | fine |
| world region list (transient at plan time) | `World::regions` | ~3 MB | fine |
| watch fingerprints | `bm-engine/src/watch/files.rs` | ~20 MB, and a `stat` of 341 k files per check interval | avg-memory/watch owner: rescan by directory mtime first |
| bundle index cache | `bm-storage` bundle cache | ≤ 4096 × ~10 KB = 40 MB, bounded | done (LRU, below) |
| lowres spills | persistence thread | CPU, not memory | Hilbert-like region order would cut spills to ~0, but changes Java's spiral progress |

**No-op `-r` / `-u` startup at scale.** Peak RSS of a no-op is flat (~105 MB at k=1..8) and no longer holds every
cell. CPU is ~0.7 ms per region (k=8: 3.4 s): extrapolated ≈ 4 min at k=73. Profile (k=4, perf): SipHash 15% (fixed
above), `Region::open` 6.4%. Each region reloads the headers of its 3×3 neighbourhood (`Headers::load`,
`bm-engine/src/actions.rs`), so every header is read 9 times. A sliding header cache along the plan order would
cut that ~9× (CPU agent's file, reported).

## Optimized storage: bundle reads at 300k² (~340 k bundles)

Bench: `cargo run -p bm-storage --release --example bundle_read_bench -- <maps root> <map> --threads 32` on the
k=8 output (4096 bundles, 7.6 GB), `read_hires_packed` as the webserver calls it. "Disk-cold": page cache dropped
with `posix_fadvise(DONTNEED)`.

| pass | base | new |
|---|---|---|
| disk-cold, first touch per bundle | 50 req/s, p99 2.1–2.6 s | 52 req/s, p99 2.0 s |
| page-warm, random bundles (3 rounds) | 175–422 k req/s | 188–426 k req/s |
| page-warm, one view (4 bundles, 32 threads) | 98–196 k req/s, p50 17–22 µs | 183–268 k req/s, p50 15 µs |

- **Index cache**: least recently used bundles are dropped (an eighth at a time, never one in use) instead of
  clearing all 4096 when full, so a hot working set survives a map 80× larger than the cache.
- **Lock**: the bundle's lock covers only the index refresh; the record is read positionally afterwards, so readers
  of one area no longer serialise on file I/O (one-view p50 22 → 15 µs).
- **Cold first touch is still I/O-bound** at ~1.9 MB per bundle on this HDD. The format has no index on disk, and
  only reading the whole log finds a tile's latest record. Tried: indexing by the 12-byte record heads alone
  (positional reads, payload checksums deferred to the record read): **22 req/s, p99 15 s**, 2× worse, because
  scattered small reads defeat readahead on an HDD. Reverted.
  Done since: a sidecar index `x…z….bmi` per bundle (spec and safety rules in `optimized/bundle/mod.rs`), written
  when a write handle closes (`MapStorage::flush` after every update pass, drop, writer eviction, compaction).
  In-log index records were rejected: an older version's scan stops at an unknown record kind and its writer
  would truncate everything after it. Cost: one more file per bundle (+340 k files at 300k²).
