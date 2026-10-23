# 23 — Profile: average memory, allocation churn, allocators

Metric: steady-state RSS (sampled over time), allocations per tile, temporary allocations, allocator CPU share, RSS
creep in long-running `-u` and in the webserver; glibc vs the shipped musl + mimalloc core. Measured on the testbox
(12 threads, 31 GiB, shared: background load 2–90 from other sessions, so CPU numbers are paired or locked runs).

## Setup

- Fixtures (`~/bmrs-avgmem/work/fx`): `real` = the 4096² real world (100 regions, 16,641 hires tiles),
  `format: optimized` file storage, forced `-r -f`, 12 render threads; `mini` = its 3×3 central regions (2,209
  tiles) for heaptrack/perf.
- Tools (all print summaries, never dumps):
  - `tools/rss_sample.py -- <cmd>`: RSS/threads sampled from `/proc` (mean/p50/p95/max + bucketed timeline,
    optional CSV), CPU from `wait4`.
  - `tools/soak_watch.py --fresh …`: `-u` soak. Seeds 16 regions nearest the origin, then every 20 s moves the next
    one in (the frontier a bot explores) and rewrites 2 random present regions with a new mtime (revisits).
  - `tools/web_load.py`: random hires/lowres tiles from packed (BMQ3 + zstd), legacy gzip and identity clients.
  - `tools/heaptrack_sites.py <zst>`: streams `heaptrack_print` backtraces (GBs, never on disk) and ranks
    `crates/bm-*` file:line by calls or temporaries.
  - `tools/perf_callers.py <leaf-regex>`: nearest workspace caller of a hot leaf in `perf script` output.

## Forced render (`real`, `-r -f`)

| build | CPU s | wall s | RSS mean | p95 | max | timeline |
|---|---|---|---|---|---|---|
| glibc (system malloc) | 293 | 122* | 713 | 868 | 877 | climbs monotonically 387 → 868 |
| musl + mimalloc (shipped, purge 3 ms) | 368 | 192* | 401 | 537 | 623 | flat 360–540 |

\* loaded box; locked medians are in "Applied".

Heaptrack, glibc, `real`: **18.36 M allocation calls = 1,103/tile, 7.87 M temporary (43%)**, peak heap 662 MB
against 877 MiB RSS. About 215 MB of the glibc peak is allocator retention, not live data. `mini`: 1,496/tile
(resource loading amortised over fewer tiles), peak heap 383 MB vs 542 MiB RSS.

## Allocator comparison (`real`, env-var variants, one run each, same box state per row group)

| variant | CPU s | sys s | RSS mean | p95 | max |
|---|---|---|---|---|---|
| musl + mi, purge 3 ms (shipped) | 374 | 36.5 | 390 | 552 | 652 |
| … purge 10 ms | 365 | 33.3 | 460 | 613 | 692 |
| … purge 30 ms | 362 | 29.3 | 507 | 666 | 725 |
| … purge 100 ms | 356 | 22.5 | 516 | 713 | 764 |
| … purge 1000 ms | 344 | 12.8 | 689 | 930 | 979 |
| glibc | 297 | 3.0 | 719 | 883 | 886 |
| glibc `MALLOC_ARENA_MAX=4` | 298 | 4.7 | 606 | 690 | 742 |
| glibc `mmap_threshold=256K` | 337 | 36.1 | 344 | 499 | 564 |

With the memset fix below, these purge knobs all lose to the shipped setting: `MINIMAL_PURGE_SIZE=1M`
320 CPU-s / 364 mean, `ALLOW_THP=0` 320 / 382, `PURGE_DELAY=10 ARENA_PURGE_MULT=3` 320 / 474, `PAGE_FULL_RETAIN=8`
332 / 406, shipped 318 / 351. The purge delay is the only real lever: each 10× buys about 3% CPU for about 30% RSS.

**User time on musl did not move with the purge delay (331–337 s vs glibc 294 s).** perf found why: the musl build
spent **11.1% of all samples in `compiler_rt.memset`** (glibc's AVX2 memset: about 3%). zig 0.16's compiler-rt
supplies libc's string functions in our static musl cores, and its `memset` is a byte-store loop (disassembly: one
`mov %sil` per byte, unrolled 8×). Callers (`perf_callers.py`, share of all samples): chunk-area collection 2.0%,
`HiresRenderer::block_pass` 1.2%, `Compression::decompress_into` 1.1%, `push_face` 1.0%, `write_prbm` 1.6% (3
sites), `render_model` 0.6%, `write_png_into` 0.6%, plus 1.9% on the single `bluemap-persist` thread. Its `memcpy`
(2.3%) is vectorised; `bcmp`/`memcmp` are also byte loops but below 0.3%.

## Applied

1. **`crates/bm-cli/src/memfns.rs`: strong `memset` for musl targets** (zig's symbols are weak). It uses overlapping
   scalar stores up to 32 B, then aligned 16-byte blocks (SSE2 on x86_64, `u128` elsewhere). The block stores are
   volatile so LLVM can't turn the loop back into a recursive `memset` call. It applies to all three Linux core
   targets. Locked, interleaved, 3 pairs on `real`:

   | | CPU s (median) | wall s (median) | RSS mean |
   |---|---|---|---|
   | musl shipped | 361.4 | 38.6 | 394 |
   | musl + memset | **339.2 (−6.1%)** | **31.4 (−19%)** | 393 |
   | glibc (same lock) | 296.2 | 27.3 | 704 |

   The musl gap to glibc drops from +22% CPU / +41% wall to +15% / +15%; what remains is mostly mimalloc purge sys
   time (31 s vs 2 s). Output is byte-identical: a compat-storage `mini` render with and without it differs only in
   the 13 rstate `.tiles.dat` files, which carry render timestamps and also differ between two runs of one binary.

2. **`alloc::release_free_memory()` = `malloc_trim(0)` when the render queue runs dry** (`bm-cli` render loop and
   plugin worker; glibc only). glibc returns only the top of the main heap on `free`; the render threads' arenas
   keep their peak forever. It is a no-op for `-r` (fires once at exit): locked 296.2 vs 297.6 CPU-s.

## Soak: `-u` for 25 min (72 regions added, 144 rewrites)

| build | CPU s (user/sys) | RSS mean | p50 | p95 | end |
|---|---|---|---|---|---|
| glibc | 286 (283/2.8) | 718 | 744 | 773 | 773 (monotonic creep, never returns) |
| glibc + idle trim | 297 (287/10.5) | **279** | 247 | 509 | 241 |
| paired rerun, glibc + trim | 336 (323/13.2) | 301 | 302 | 475 | 304 |
| paired rerun, musl + memset | 376 (338/38.2) | 243 | 239 | 270 | 228 |
| musl, `mi_collect(true)` at idle | 366 (328/37.8) | 244 | 235 | 304 | 219 |
| musl, no collect (same pair) | 365 (327/38.0) | 240 | 233 | 304 | 213 |

- glibc without trim creeps to its forced-render peak and keeps it. Trim cuts mean RSS 61% (718 → 279 MiB) for
  +11 CPU-s per 25 min (+3.8%), all sys time from re-faulting the returned pages on the next update.
- musl + mimalloc does not creep at all; an idle `mi_collect` changes nothing, so the shipped core gets no hook.
- Paired, musl costs 12% more CPU than glibc + trim for 19% less mean RSS in watch mode.

## Webserver soak (`-w` on `real`, 10 min of tile load then 2 min idle, ~1,900 req/s, all 200)

| build | CPU s | RSS mean | p95 | max | after load | threads |
|---|---|---|---|---|---|---|
| glibc, 8 clients | 312 | 80 | 85 | 85 | 84 | 39 |
| musl, 8 clients (paired) | 350 | 91 | 115 | 124 | 62 | 42 |
| musl, 16 clients | 336 | 135 | 155 | 159 | 113 | 47 |
| musl, 16 clients, `mi_collect` every 5 s | 337 | 147 | 196 | 200 | 115 | 49 |

glibc sits flat at about 85 MiB. musl is bounded but noisy: two earlier unpaired 8-client runs peaked at 251 and
304 MiB, and one kept 206 MiB after the load stopped. The blocking-pool thread count is stable (no thread churn), and
a periodic `mi_collect` doesn't help. No fix applied; the server's footprint is small next to a render's.

## Allocation churn by call site (heaptrack, glibc, `real`, per hires tile)

| calls/tile | temp/tile | site (file:line) | what | owner |
|---|---|---|---|---|
| 471 | 269 | bm-world/src/chunk/modern.rs:53 | `collect_sections` + inlined `parse_section`: per-section `Light::from_nibbles` boxes, palette `Box<[StateId]>`, `Padded` indices `Vec` | µarch |
| 149 | 47 | bm-world/src/chunk/modern.rs:86 | palette `collect()` per section | µarch |
| 79 + 19 | 66 | bm-world/src/chunk/modern.rs:123,122 | `pairs: Vec<(&str,&str)>` per propertied palette entry, just for the lookup | µarch |
| 36 + 9 + 2 | 33 + 9 + 2 | bm-world/src/registry.rs:205,203,204 | `state_key` builds a `String` per lookup (≥95% temporary) | µarch |
| 34 + 5 | 14 | bm-compress/src/lib.rs:241,168 | `read_capped`/`decompress_into`: chunk output `Vec` grows by `read_to_end` doubling | µarch |
| 16 + 8 | 0 | bm-world/src/registry.rs:42,45 | `snapshot()` clones all of `StatesInner` (two `FxHashMap<Box<str>,_>`) per region (`bm-engine/src/update.rs:136`): about 3.9k allocations per region | µarch / CPU |
| 95 | 0 | bm-render/src/mesh.rs:52–59 | `push_face`: 7 `TileModel` vecs grow by doubling on every tile | CPU |
| 10 | 0 | bm-world/src/world.rs:123 | chunk-area load | µarch |
| 7 + 5 | 4 + 3 | bm-storage/src/optimized/bundle/mod.rs:109 | `path()` formats a `PathBuf` per append / versioned read | peak-RAM |
| 8 | 1.5 | bm-format/src/compact/codec.rs:76 | `encode_into::body_into` | µarch |
| 6 | 0.3 | bm-render/src/mesh.rs:108 | `sort_by_material::gather` | CPU |

The chunk parser alone is about 70% of all allocation calls and 88% of temporaries. Suggested fixes (owners decide):
- Per-chunk arena or flat buffers for palettes and indices.
- A `Light::Uniform` variant for all-0/all-15 sections.
- A `&mut String` scratch in `ChunkContext` for `state_key`, and a reused `pairs` scratch.
- Decompress into a reused per-thread buffer.
- A shared, copy-on-write registry snapshot.
- Reuse `TileModel` capacity across tiles.

Zero-then-overwrite memsets (`decompress_into`, `write_prbm`, `push_face` resizes) cost about 1% on glibc once
memset is fast; low priority.

## Other notes

- **Watcher rescan** (`bm-engine/src/watch/files.rs`): notify events are primary. The fingerprint rescan runs every
  `region-file-check-interval` (default 5 min) as the lost-event fallback. One stat pass over the 4,096-region scale
  world takes about 10 ms warm, so at 300k² (about 341k regions) that is about 0.85 s per 5 min (0.3% of a core)
  plus about 16 MB of fingerprints. Left as is.
- Rejected:
  - `MALLOC_ARENA_MAX=4` (−16% render RSS at no CPU cost) was not applied: it is weaker than trim for `-u`, and glibc
    builds are not the shipped core.
  - `mmap_threshold=256K` cuts RSS 52% but costs +13% CPU.
  - Periodic `mi_collect` in the webserver: no effect.
