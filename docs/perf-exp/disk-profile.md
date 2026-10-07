# Disk footprint and storage-write cost (bluemap-rs vs Java 5.28)

## Setup
- Machine: Core Ultra 7 155H (hybrid, 22 threads), NVMe, NTFS with 4 KiB clusters, Defender real-time protection on.
  Another agent was building in the same target dir, so timings use the min of 3 runs per tile where noted.
- Data: `work/bench/{vanilla-512,structures}-{java,rs}/web` (forced renders, ba05002 bench), golden lowres PNGs from
  `work/bluemap/*/web` (380 tiles).
- Scripts and examples (no product src changed):
  - `py -3 -I docs/perf-exp/disk_inventory.py <webroot>... [A=B]` gives logical bytes plus real NTFS allocation
    (`GetFileInformationByHandleEx`) and per-file Java/Rust diffs.
  - `cargo run -p bm-compress --profile profiling --example codec_bench -- <maps dir> [--every N --slow-every M --reps R --only a,b]`
  - `cargo run -p bm-format --profile profiling --example png_bench -- <dir>...`
  - `cargo run -p bm-storage --profile profiling --example write_bench -- <maps/<id>/tiles/0> <scratch> --threads 8`
- Render CPU baseline (`docs/results/bench.jsonl`, ba05002): Rust spends **109–111 ms CPU per hires tile** end to end
  (structures 126 CPU-s / 1153 tiles, vanilla-512 32 CPU-s / 289 tiles).

## 1. Disk breakdown (map data only; the webapp is excluded)
| category | vanilla-512 Java | vanilla-512 Rust | structures Java | structures Rust |
|---|---|---|---|---|
| hires `.prbm.gz` (n=289 / 1153) | 19,903,452 | 19,756,319 (0.993) | 81,005,221 | 80,294,370 (0.991) |
| textures.json.gz | 587,926 | 484,090 (0.823) | 587,926 | 484,090 (0.823) |
| lowres LOD1 / 2 / 3 PNG | 180,171 / 34,300 / 11,869 | 186,592 / 34,116 / 15,038 | 712,339 / 105,106 / 19,703 | 680,862 / 93,288 / 22,866 |
| rstate tiles / chunks / regions | 1,054 / 2,444 / 353 | 786 / 2,392 / 357 | 3,893 / 7,548 / 477 | 2,919 / 7,246 / 439 |
| settings (root+map), live json | 746 / 4 | 746 / 4 | 740 / 4 | 740 / 4 |
| **total logical** | **20,722,315** | **20,480,436 (0.988)** | **82,442,957** | **81,586,824 (0.990)** |
| **total allocated (NTFS)** | 21,366,944 (×1.031) | 21,104,528 (×1.030) | 84,898,904 (×1.030) | 84,045,912 (×1.030) |
| files / dirs under `maps/` | 334 / 40 | 334 / 40 | 1247 / 242 | 1247 / 242 |

- Hires is 96–98% of the bytes. Every other category together is under 4%.
- **Allocation overhead:**
  - hires 1.029×
  - lowres 1.09–1.66×
  - `chunks.dat` 2.2–6.8×: ~600 B files are non-resident, so each takes a full 4 KiB cluster
  - other files under ~700 B are MFT-resident (alloc ≈ size)

  Overall +3.0%, the same for Java and Rust.
- **Rust vs Java per file:**
  - All 1442 hires tiles decompress to identical PRBM, but **0** are byte-identical: zlib-rs level 6 vs Java's C zlib,
    0.7–0.9% smaller. C zlib 1.3.1 level 6 reproduces Java's stored bytes exactly.
  - All 95 lowres PNGs are pixel-identical, at 1.042× Java (vanilla) and 0.952× (structures).
  - textures.json has identical entries, with its embedded PNGs re-encoded to 0.35× (1,557,267 → 544,657 B).

## 2. Compression cost per tile (single thread; raw PRBM averages 1.21 MB)
vanilla-512, 145 tiles (every 2nd), min of 3 runs. Slow codecs (★) ran on 25 tiles. Structures numbers (every 4th
tile) match within ±0.01 in ratio.

| codec | size vs Java gz | enc ms/tile | dec ms/tile |
|---|---|---|---|
| gzip-1 (zlib-rs) | 2.894 | 2.6 | |
| gzip-3 | 1.151 | 4.1 | |
| gzip-4 | 1.058 | 5.8 (6.3 structures) | |
| **gzip-5** | **0.997** | **7.9 (8.6)** | |
| **gzip-6 = product `Gzip`** | **0.992** | **14.7 (15.8)**; 24 under 8-thread load | 1.1–1.7 |
| gzip-9 ★ | 0.999 | 110–170 | |
| Java / C zlib gzip-6 (Python zlib, proxy) | 1.000 | ~75 (one run, contended) | |
| lz4-java (product) | 3.87 | 2.8 | 1.7 |
| zstd-3 product (`copy_encode` stream) | 0.972 | 3.0–3.2 | 2.3–2.7 (stream reader) |
| zstd-3, reused bulk context | 0.972 | 2.3–2.8 | 1.9–2.0 (bulk) |
| zstd-6 / 9 / 12 ctx | 0.902 / 0.897 / 0.895 | 7.3–8.0 / 12 / 27 | 1.2–1.5 |
| zstd-15 / 16 / 18 / 19 ctx ★ | 0.897 / 0.705 / 0.635 / 0.627 | 67 / 115–123 / 202 / 290 | 1.3–2.0 |
| zstd-3 / 6 + long-distance matching (w24) | 0.944–0.952 / 0.892 | 3.4–4.1 / 8.3–9.0 | 1.3–1.9 |

- The size cliff sits at the switch to optimal parsing (level 16, btopt). Cheaper btopt/btultra parameter sets
  (search log 1–4, min-match 3) gave 0.78–0.83 at 50–96 ms (Python `zstandard` probe on 25 tiles), so there is no
  cheap route to the 0.63 of level 19.
- In compat mode the *format* is fixed: gzip unless the user configured another compression. The exact bytes are
  not fixed, because Rust already differs from Java on every tile, so the level can change.

### Lowres PNG (`png_bench`, 380 golden tiles, 2,224,441 B stored by Java)
| setting | size vs Java | enc ms/tile |
|---|---|---|
| **product `encode_png`** (`Compression::High` = zlib 9 + Adaptive) | 1.014 | **19.6** |
| same settings without the RGBA conversion in `encode_png` | 1.014 | 13.3 |
| zlib 6, None | 1.106 | 2.4 |
| **zlib 9, None** | **0.843** | **6.8** |
| zlib 9, Up | 1.017 | 13.4 |
| fdeflate (png Fast / Fastest) | 2.9–3.2 | 0.5–1.1 |

`decode_png` takes 3.5 ms/tile.

## 3. Storage write paths (`write_bench`, vanilla-512, 289 tiles, 68 KB gz / 1.21 MB raw)
| op | thr | tiles/s | user ms | kernel ms | allocs | alloc KiB |
|---|---|---|---|---|---|---|
| file `write_grid` gzip (fresh) | 1 | 48 | 18.7 | 2.0 | 30 | 408 |
| gzip-6 compress only | 1 | 62 | 15.7 | 0.3 | 6.5 | 561 |
| file `write_grid_encoded` (overwrite, atomic) | 1 | **425** | 0.0 | 2.2 | 25 | 3 |
| plain `fs::write` (overwrite, not atomic) | 1 | **1594** | 0.0 | 0.3 | 15 | 2 |
| file `write_grid` zstd-3 (fresh) | 1 | 162 | 3.6 | 2.5 | 29 | 39 |
| file `write_grid` gzip (fresh) | 8 | 307 | 22.7 | 1.9 | 31 | 421 |
| file `write_grid_encoded` (overwrite) | 8 | 879 | 0.7 | 1.2 | 25 | 3 |
| plain `fs::write` (overwrite) | 8 | 3219 | 0.1 | 0.7 | 15 | 2 |
| file `write_grid` zstd-3 (fresh) | 8 | 458 | 5.4 | 5.6 | 29 | 51 |
| sqlite `write_grid_encoded` fresh / overwrite | 1 | 1102 / 803 | 0.4 | 0.6–0.9 | 27 | 72 |
| sqlite `write_grid_encoded` fresh / overwrite | 8 | 1023 / 1082 | ≤0.3 | 0.4 | 27–30 | 72 |
| sqlite one transaction, raw sqlx REPLACE | 1 | 942 writes/s | 0.3 | 0.7 | 11 | 68 |

- **Atomic write steps** (`fsops::write_atomic`, fresh targets, µs per write): `create_new` open 600, write 193,
  close 114, rename 821. That adds about 1.7 ms of kernel time per write over a plain overwrite. 8 threads give only
  2.1× the throughput, because the directory and Defender serialize.
- **No throughput limit at current render speed:** the render runs at ~80–90 tiles/s; atomic writes cap at 879/s
  and SQLite at ~1000/s.
- **SQLite:**
  - WAL + `synchronous=NORMAL` already makes each commit cheap, so batching gives nothing (942 vs 803–1102/s).
  - The database file is 20.07 MB for 19.76 MB of blobs (+1.6%; file storage +2.9%).
  - Writes don't scale with threads: there is a single writer.
  - MySQL and PostgreSQL round trips were not measured.
- **Allocation:** a gzip `write_grid` allocates ~408 KiB per tile. That is zlib-rs deflate state built fresh for each
  `GzEncoder`. The rest is ~25 small path and string allocations.

## 4. Ranked hotspots
| # | Where | Cost | Share |
|---|---|---|---|
| 1 | Hires gzip-6 on render threads: `crates/bm-compress/src/lib.rs:36` (`DEFLATE_LEVEL = 6`), `:118`, via `crates/bm-storage/src/api.rs:115-127` (`with_encoded`) | 14.7–15.8 ms/tile single-thread, ~24 ms under parallel load | **~14–22% of total Rust render CPU** (110 ms/tile) |
| 2 | Missing `optimized` storage format: `format: optimized` parses (`crates/bm-config/src/config/storage.rs:23,108`) and is the generated default (`crates/bm-config/templates/storage-format.conf:8`), but bm-storage has no such layout and bm-engine (branch ba05002) never reads it | New installs silently get compat gzip | Disk: the only big lever (09: BMQ1+zstd 0.133×; plain zstd-19 0.63×) |
| 3 | Re-writing unchanged content: hires (gzip + atomic write), lowres (encode + write), all rewritten even when bytes are identical (forced renders, incremental passes over unchanged tiles) | ~17 ms/tile hires, ~20 ms per lowres tile | Up to ~15% of CPU on a forced re-render of an unchanged world |
| 4 | Lowres PNG encode: `crates/bm-format/src/lowres.rs:64` (`Compression::High` = zlib 9 + Adaptive) and `:66` (`flat_map().collect()` into a fresh 2 MB `Vec`) | 19.6 ms/tile on the **single** persistence thread | <1% CPU; serial; lowres bytes 1.01× Java |
| 5 | Atomic file write: `crates/bm-storage/src/file/fsops.rs:70-88` | ~1.7 ms kernel per write above a plain write | ~2% of render CPU; ~40–60% of storage cost if zstd-3 is used |
| 6 | zstd path: `lib.rs:125` builds a stream CCtx per call; `lib.rs:147-149` decodes through a stream reader | 3.0–3.2 vs 2.3–2.8 ms encode; 2.3–2.7 vs 1.9 ms decode | Only when zstd is configured, or for web transcodes |
| 7 | zlib-rs state allocated per gzip call (`lib.rs:118,122`) | 408 KiB alloc + init per tile | <1% CPU |
| 8 | SQL key cache: `crates/bm-storage/src/sql/keys.rs:36` (`key.to_owned()` per lookup, ×3 per write) plus `GridKey::sql_key()` String | a few allocations per write | negligible |
| — | rstate, settings and live JSON; textures.json rewritten per map load (bm-engine `map.rs:138`, ~1.15 MB gzip) | <0.1% of bytes; ~15 ms per map load | not worth it |

## 5. Fixes (C = compat-safe, same format; O = optimized mode only)
1. **C: hires gzip level 6 → 5** (`DEFLATE_LEVEL`).
   - Size 0.997× Java (vs 0.992× now), still not larger than Java.
   - gzip CPU −46% (14.7 → 7.9 ms), which is **~6–7% of total render CPU**.
   - Hires disk +0.5% vs current Rust.
   - Level 4 gives −60% CPU at 1.058× size; that is not recommended, since it is larger than Java.
2. **C: lowres PNG `Level(9)` + `Filter::NoFilter`**, and build the RGBA buffer with exact capacity (or reuse a scratch
   buffer).
   - 19.6 → ~6.8 ms/tile (−65%).
   - **−17% lowres bytes** (0.843× Java). Pixels are unchanged, and lowres.rs already allows other PNG bytes.
   - Matches doc 11 #5.
3. **C: skip unchanged writes.**
   - Hires: keep an xxh3 of the raw PRBM per tile (in memory per session; `twox-hash` is already a bm-compress dep)
     and skip compress and write on a match. That saves ~17 ms per unchanged tile.
   - Without a hash cache, read + gunzip + compare costs ~1.5 ms and is worth it only when more than ~10% of
     rewrites are unchanged.
   - Lowres: keep the loaded pixels in `LowresTileManager::write` (`crates/bm-map/src/lowres/mod.rs:157-168`) and
     skip `save` on equality.
4. **O: implement `optimized`, or stop generating it as the default until it exists** (#2).
   - Interim disk-only option inside compat: users who set `compression: zstd` get 0.97× at 2.3–2.8 ms. Java reads
     any zstd level, so a configurable zstd level is compat-safe: zstd-6 is 0.90× at 7–8 ms, still half of gzip-6.
   - zstd-19 (0.63×, ~290 ms/tile) only suits background recompaction, never the render path.
5. **C: reuse codec contexts per thread.** Use a thread-local `flate2::Compress` with `reset()` and a
   `zstd::bulk::Compressor`/`Decompressor`.
   - zstd: −7…27% encode, −15–30% decode.
   - gzip: removes 0.4 MB of allocation per tile.
   - zstd frames gain a content-size field, which is still the zstd format.
6. **C (low priority): atomic write.**
   - The 1.7 ms/write is mostly Windows create + rename (Defender on). It is only worth attacking after #1/#3 make
     compression cheap.
   - Options: skip writes as in #3; don't fall back to `create_dir_all` per new directory (already done only on
     NotFound).
   - Durability semantics must stay the same: no torn tiles.
7. **SQL: no change needed for SQLite.** Measure MySQL/PostgreSQL round trips before adding multi-row batching
   (doc 11 §1.6).

Expected combined (compat): about −7% render CPU from #1, −65% persistence-thread lowres time and −17% lowres bytes
from #2, and ~15% on forced re-renders of unchanged areas from #3. Hires disk stays at 0.997× Java. Big disk savings
need #4.

## Caveats
- Timings are on a hybrid laptop CPU with a concurrent cargo build. Single-thread numbers use min-of-3; multi-thread
  user-CPU per tile is inflated by E-cores and HT. The C-zlib proxy figure is a single contended run.
- The fixtures are small and partly synthetic (see 09/11). Lowres and rstate shares will be larger on real survival
  worlds, but stay small next to hires.
- Defender adds to every file create and rename. Excluded folders or Linux will show a smaller atomic-write gap.
