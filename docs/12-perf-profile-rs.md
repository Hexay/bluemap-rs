# 12 — Profiling bluemap-rs: CPU, memory, wall, RSS, disk

Profiled 2026-10-07 on master `ffd037a` (`render_map` harness) and engine branch `ba05002` (`bluemap -r -f`, the
real pipeline). Deep dives: [perf-exp/disk-profile.md](perf-exp/disk-profile.md) (storage, compression, PNG),
[perf-exp/web-profile.md](perf-exp/web-profile.md) (bm-web under load).

## Setup

- samply needs an elevated ETW helper on Windows (UAC prompt), so sampling ran on **testbox** (12 threads, Linux,
  `perf` + `heaptrack`, `perf_event_paranoid=1`). Repo shipped as a git bundle; fixture `structures` (1153 tiles,
  16.5 M faces, 17 145 chunks).
- Runners: `docs/perf-exp/testbox_harness.sh` (render_map), `testbox_engine.sh` (bm-cli forced render), modes
  `plain|perf|heaptrack`. Rank folded stacks with `docs/perf-exp/folded.py <folded> self|incl|callers <re>`.
- Gotcha: heaptrack `-F` folded exports are 6–8 GB here (deep inline stacks). Aggregate on the box, never scp.

## Numbers (structures, forced full render)

| run | wall | CPU (user) | peak RSS |
|---|---|---|---|
| Java 5.28, Windows 22 thr (bench.jsonl `ba05002`) | 86.9 s | 984 s | 1238 MB |
| engine, Windows 22 thr (bench.jsonl `ba05002`) | 14.7 s | 126 s | 345 MB |
| engine, testbox 12 thr | 7.9–9.1 s | 77.4 s | 426–436 MiB |
| render_map harness, Windows 22 thr | 9.3 s | 111 s | 1887 MB |
| render_map harness, testbox 12 thr | 6.8 s | 74.4 s | 1925 MiB |
| harness, `-C target-cpu=x86-64-v3` | — | 70.3 s (−5.3%) | same |

## CPU (engine, inclusive share of all samples)

| area | share | notes |
|---|---|---|
| `HiresRenderer::render_tile` | 64.9% | `block_pass::render` 52.5% (`Renderer::face` 25.7%, liquids 3.9%) |
| ↳ `Volume::fill` | **11.3%** | `Masking::read` → `Chunk::block`/`light` per block, incl. all-air sections |
| ↳ `java_round` | **6.3%** | f64 `floor` lowers to a libm call: baseline x86-64 has no SSE4.1 `roundsd` |
| hires gzip-6 (`compress_into`) | **22–23%** | zlib-rs `longest_match` alone 16% |
| lowres (`save_and_cascade` + PNG) | 1.5% | on the persist thread |
| region read + chunk parse | ~2.5% | not worth optimising (same as Java, see 10) |
| PRBM write | 1.4% | |

Threads: 97.5% `bluemap-render-*`, 1.9% `bluemap-persist`. Timeline: ~11/12 cores busy from 0.5 s to 7.5 s.
The wall clock is CPU-bound, so CPU wins become wall wins one-for-one.

## Memory

- **Engine:** 345 MB on Windows, ~430 MiB on testbox. Fine.
- **Harness peak (1.9 GB):** 78% of it is `write_prbm` buffers. `fixture.rs::render` collects every uncompressed
  PRBM (99 B/face × 16.5 M faces = 1.63 GB) before gzipping. This is a harness artifact; the engine streams.
- **Chunk area:** ~13% of harness peak, ~270 MB, ~16 KB/chunk. `Light::from_nibbles` boxes 2 KiB per section even
  when uniform (2.2% of peak).
- **Allocation count:** chunk parsing accounts for 38% of 3.7 M allocation calls (`parse_section` palettes,
  `Padded` copy, per-entry property `Vec` and `state_key` `String`). Cheap in CPU terms, ~1% each.

## Ranked hotspots → fixes

1. **Hires gzip — 22% CPU.** Level 5 cuts gzip CPU 46% for +0.5% bytes (still 0.997× Java). Compat only pins
   the format, since zlib-rs bytes already differ from Java's. Also: offer a zstd level, reuse a thread-local
   compressor, and skip rewriting tiles whose PRBM hash is unchanged (≈15% on forced re-renders). See disk-profile §1/§3/§6.
2. **`Volume::fill` — 11% CPU.** Fill per section run instead of per block:
   - Single-palette sections and absent/uniform light become one `fill`.
   - Paletted sections decode 16 y-values per column with one section lookup.
   - The mask is only tested when `Masking.mask` is `Some`.
   - Expect most of the 11% to go; unmeasured.
3. **`java_round` soft floor — 6% CPU.** Replace `(f64 + 0.5).floor() as i32` with an integer floor:
   `let d = f64::from(v) + 0.5; let t = d as i64; (t - i64::from((t as f64) > d)) as i32`. This is exact for
   floats in i32 range. Measured upper bound: v3 build −5.3% total CPU; shipping v3 would drop pre-Haswell
   CPUs, so prefer the code fix.
4. **Per-face neighbour/light lookups (~20% self across `View::state`, `Bounds::index`, `Block::new/neighbor`,
   `relative`).** Each face re-resolves its cullface neighbour and light. Caching a block's 6 neighbours
   `(StateId, light)` once per block is the next structural win after 2–3; unmeasured.
5. **Wall-clock edges — ~1 s of 8.** Serial resource load at start (0.25–0.6 s) and a tail from region-sized
   work units (~0.5 s at 3–4 busy cores). Split the last regions into tiles, or order regions largest-first.
6. **Disk.** Hires is 96–98% of bytes, and Rust matches Java (0.99×).
   - **`format: optimized` is generated as the default but not implemented**, so new installs silently get compat
     gzip. Implement it (0.133× in doc 09) or stop defaulting to it.
   - Lowres PNG with no filter: −17% bytes, −65% encode time, pixels unchanged (`bm-format/src/lowres.rs:64`).
   - Atomic writes run at 425/s, low priority.
7. **Web.** Static-file 304s are answered after three blocking fs calls (`static_files.rs:45-72`), costing more than a
   tile. Map data has no ETag (reload re-sends ~4.3 MB). The 1.2 MB JS is served uncompressed. `textures.json` costs
   17 ms and 8.9 MB to re-encode per non-gzip client, with no memory cap. See web-profile.

## Applied

Items 2 and 3 landed together: `Chunk::column_into` plus `Masking::edge_column`, and the integer `java_round`.
Engine, structures, testbox, 3 interleaved runs each:

| | CPU | wall | RSS |
|---|---|---|---|
| before | 77.2–77.4 s | 8.0–8.2 s | 420–441 MB |
| after | 68.9–69.0 s (**−10.8%**) | 7.2–7.4 s | 411–438 MB |

Hires and lowres output are byte-identical to the previous build. Only `rstate` `.dat` differs, and it differs
between any two runs. The golden suite and a `diff-render` of `structures` against Java show 0 differences.

Item 1 then landed: `DEFLATE_LEVEL` 6 → 5 (`bm-compress/src/lib.rs`). This drops byte-identity with Java's
Deflater, which zlib-rs never had; madler zlib at level 6, as `bm-java` uses for PNGs, would be the route back.

| | CPU | wall | hires bytes |
|---|---|---|---|
| level 6 | 68.8–68.9 s | 7.1–7.4 s | 80.29 MB |
| level 5 | 62.1–62.2 s (**−9.8%**) | 6.6–6.9 s | 80.72 MB (+0.53%) |

Item 4 then landed (`a01b13a`). Rotated offsets are precomputed per variant (`relative.rs`, 27 entries), and
neighbour reads within ±1 index the tile volume directly from the block's own index. A lazy 26-neighbour cache
wasn't built, because what remained after this is spread thin across memory reads. Result: 62.1 → 55.0 s CPU
(**−11.5%**), 6.7 → 6.0–6.5 s wall, byte-identical.

Cumulative: 77.3 → 55.0 s CPU (**−29%**), 8.1 → ~6.3 s wall.

Web (item 7, merge `1a2a3cb`): `static_cache.rs` keeps static files in memory, re-checks disk metadata at most
once a second, and gzips text types once. Gzipped replies use the Java ETag plus `-gzip`. `transcode.rs` caches
converted map data, with a per-core cap. Map-data ETags: see the end of this section. Measured CPU per request:

| request | before → after |
|---|---|
| 1.2 MB JS | 7.9 → 0.48 ms, sent at 0.31 MB gzipped |
| static 304 | 1.2 → 0.19 ms |
| `textures.json` to a client without gzip | 18.6 → 2.7 ms; peak memory 132 → 89 MB |

Java conformance: 110 of 110 identical.

Map-data validators (`bm-web/src/validators.rs`, `map_data.rs`; `MapStorage::{grid,item}_version`): strong
quoted ETag = storage `Version` + body coding (`-gzip` etc. per representation); `If-None-Match` → 304 from metadata
only. Version: compat files mtime + length + file id (NTFS file index / inode; ids change on every rename-over, mtime
alone repeated within ~1 ms); optimized hires bundle generation + record offset; SQL none (Java's schema has no
change column). `ETag` is on by default (hidden `webserver.conf` key `map-etags: false` turns it off).
The conformance suite skips our `ETag`/`Vary` only where Java's reply has none. 304s are answered either way. `structures` reload (`web_bench.py --only
reload_hires,reload_map_data --server-arg=--etags`): 70 KB → 0 B body per hires tile, ~4.3 MB → 0 B body per
view (header bytes unmeasured), CPU per hires revalidation 0.60–0.68 → 0.36–0.40 ms; full replies unchanged within noise (one handle gives
bytes and version).

Disk (merge `16568e8`):
- **Lowres PNG**, zlib 9 with no filter: 1.176× → 0.869× Java's bytes (−26%), pixels identical, encode only −13%.
  The −65% encode time and the 1.014× baseline in disk-profile.md did not reproduce.
- **Unchanged hires tiles** are read back and their decoded PRBM compared; on a match the write is skipped.
  Unchanged lowres tiles are detected with an xxh3 of their pixels. Tile events still fire, as in Java.
  Re-render over identical output, wall: compat 12.9 → 4.8 s (ext4 flushes data on rename-over),
  optimized 5.7 → 4.8 s.
  Cost when every stored tile changed: +4% compat, +9% optimized. Empty storage costs nothing.
  If forced re-renders after a settings change matter more, gate the read-back on `-f` plus a changed map config.

## Round 3 (re-profiled master `4a8654d`)

Starting point on testbox: compat 53.4 s CPU / 5.7 s / ~435 MB / 84 MB disk, optimized 54.3 s / 5.7 s / ~675 MB /
15 MB. Compat hotspots: gzip 20%, `Block::neighbor` 10.5%, `Block::new` 9.5% (every y, air included), liquids 5.4%,
`Volume::fill` 5.4%.

| commit | change | effect (structures, testbox, 3 interleaved runs) |
|---|---|---|
| `a44b23f` | block pass: interior air only feeds its light to the column | CPU 53.5 → 49.1 s (−8.3%), webroot identical |
| `7d91907` | gzip/zlib encode via libdeflate 4 (decode stays flate2) | CPU 53.6 → 50.4 s (−6%), bytes 0.987× Java (was 0.997×), RSS +~8 MB |
| `f9a4bd2` | BMQ2 encoder: no release trial decode, no gathered copies, trimmed scratch | optimized RSS 670 → 472–493 MB, CPU −5%, blobs identical |

- **libdeflate output sizing:** it needs the whole output buffer up front, and sizing it to the bound kept every
  buffer at input size (+50 MB RSS). The output now starts from a guess (`len/8`), retries at the full bound on
  `InsufficientSpace`, and is shrunk to fit.
- **BMQ2 exactness without a trial decode:** the argument is in the `compact/mod.rs` "Exactness" docs. Debug builds
  (and so the unit tests) still decode and compare every blob, and `oracle_optimized` passes.
- **Next candidates:** `Block::neighbor` (~10%), liquids (5%), `Volume::fill` (5%), and gzip, still ~13% after
  libdeflate. Compat-mode gzip can't drop further without a format change, which is what optimized storage is.

## Round 4 (render CPU, from master `82ae6a5`)

Structures on testbox, 3 interleaved runs, each commit measured against the previous one. In total, compat CPU fell
47.7–48.0 s → 27.0–27.1 s (−44%) and wall 5.2–5.6 s → 3.4–3.6 s.

| commit | change | CPU |
|---|---|---|
| `0304edd` | neighbour reads via one flag byte per state; rotated offsets carry a precomputed volume step | −4.5% |
| `42d088c` | air/water/waterlogged bits copied into `StateInfo`; variant weights contiguous for the pick | −2.4% |
| `408a453` | buried blocks: a state whose faces all have in-range cullfaces, every one culled, is skipped like air | −29% |
| `4c67d27` | submerged plain water skipped the same way (`flags.rs`) | −7.5% |
| `208edd3` | tints: if the 5×3×5 blend box is one biome, one cached blend per tile, state and biome | −6.6% |

- **Skips stay exact:** debug builds re-render every skipped block and assert it adds no faces and no colour. They
  also compare every cached tint with the full blend. The debug golden run exercises both. Swamp grass (noise) and
  1.13–1.14 per-column biomes always take the full blend.
- **Tried and dropped** (noise is ±1%):
  - per-face precomputed tables: +3%, because each table entry is several times larger
  - division-free palette unpacking, per column or per section: +1.5%; the fill is bound by memory traffic
  - interleaving state and light: +1.5%
  - caching AO neighbours, forced inlining, gathering corners after culling: +1–2.4%
  - running-max column light, per-biome colour cache inside the blend, reordered cullface checks: neutral
- **Left:** compat gzip ~30% (fixed by the format), `Volume::fill` ~10%, face meshing ~11%.
