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

## Round 5 (render CPU, from master `df7184e`)

Structures on testbox, 6 interleaved runs, each commit measured against the previous one. Compat CPU fell
26.5 s → 20.3 s (−23%); optimized storage gained another −8.6% from the BMQ2 encoder.

| commit | change | CPU |
|---|---|---|
| `15c2a4b` | volume filled only up to the tile's highest non-air section; air above read for its block light only | −11% |
| `df3542a` | material sort gathers into sized buffers; `write_prbm` writes attributes in place | −5.3% |
| `020a6c5` | column reads: palette indices and light nibbles stepped without division | −2.7% |
| `d3075b6` | BMQ2 `quantize` rounds without libm (`f64::round` is a call on baseline x86-64) | −4.8% optimized |
| `a13a430` | dark cave blocks (removed as cave, block and face-light neighbours unlit) skipped like buried ones | −4.2% |
| `3a28295` | BMQ2: on-grid values skip the rounding path | −4.0% optimized |
| `801c2b3` | lowres column light: u8 light range per column alpha instead of a float max per block | −2.8% |

- Every skip has a debug path: skipped blocks are re-rendered and asserted empty, the column light and the fast
  quantize are checked against the per-block and f64 versions (golden debug run plus a debug render of structures).
- **Tried and dropped:**
  - per-thread pool for `TileBuffers` (engine makes one per rayon split): page faults unchanged, ±noise
  - one culling bit per volume block for the buried test: +5%; building it costs more than the neighbour reads
  - solid sections (palette all culling cubes) skipped without neighbour reads: +3–5%; few qualify, scans cost
  - `push_quad` (one capacity check per attribute per face pair): −1%
- **Left (compat):** gzip ~38%, block pass ~30% (per-block state/flags/light loads, spread thin), chunk load
  ~8.5% (zlib inflate ~4%, NBT walk ~4%), fill ~6%, face meshing ~8%, lowres persist ~4.6%, sort + PRBM ~6.5%.
  Optimized: BMQ2 encode is now mostly zstd.

## Lowres-only path (`enable-hires: false`)

Maps without hires tiles still meshed every tile and threw the model away (Java does the same). With hires off,
`render_top_only` is forced too. `HiresRenderer::render_lowres` (`bm-render/src/renderer.rs`) now runs the same
block pass with `Ctx::geometry` false:

- Faces pass every cull, light and cave test and set the tint and map colour as before. They are then only counted
  (`MeshExt::count_faces`), so the 1 M-face cut-off truncates at the same column. There are no UVs, AO, vertices,
  transforms, material sort or PRBM.
- Top-only walks stop at the first opaque culling block, so the volume starts 4 blocks under the tile's lowest
  `OCEAN_FLOOR`. A column that walks below that floor returns `Stop::Shallow`, and the tile refills from the bottom
  (40 of 1153 structures tiles). Masked tiles always fill fully, since masked-out blocks don't stop a walk.
- Exactness: debug builds render every lowres-only tile the full way as well and assert identical columns and
  `truncated`. The golden test renders all 6 fixtures lowres-only, with the fixture settings and with
  `render_top_only`. It compares against the full path and the golden LOD 1 pixels. On structures with hires off, the
  webroot from master's binary and from this one differ only in rstate.

Structures with `enable-hires: false`, testbox, 5 interleaved runs:

| | CPU | wall | RSS |
|---|---|---|---|
| master | 7.03–7.09 s | 1.68–1.81 s | 219–224 MB |
| no geometry, full volume | 6.33–6.43 s (−10%) | 1.63–1.67 s | 211–214 MB |
| plus floored volume | 5.48–5.52 s (**−22%**) | 1.57–1.60 s (−9%) | 204–206 MB |

Hires on (6 runs): 20.09–20.27 → 20.23–20.30 s, within noise.

- **Tried and dropped** (each cost hires CPU):
  - a generic mesh (`impl MeshExt` plus a counting type): +3–6%. The second instantiation of the block pass changed
    inlining in the first, even with `#[inline(always)]` on the helpers that got outlined.
  - an enum mesh (`Model(&mut TileModel) | Count`): +5.7%. The `&mut` inside the enum loses `noalias`, so the walk
    reloads state after every push.
  - a per-block `y < lowest` check in the walk: +3%. It is now one range bound and one flag per column.
- **Left (hires off, profiled before the floor):** chunk load ~28%, block pass ~22%, fill ~19%, lowres persist ~14%.

## Wall clock (startup, region overlap, tail)

Before this round, structures used 27 s of CPU on 12 threads but took 3.4 s of wall time, where 27/12 would be
2.25 s. The busy-core timeline (`docs/perf-exp/busy_timeline.sh`) showed three gaps:
- about 0.45 s of mostly serial startup
- a dip to 5–7 busy cores at every region boundary
- a 0.45 s tail at 1–1.5 cores, with the persist thread encoding PNGs alone

| commit | change | wall (median, testbox) |
|---|---|---|
| `adfd42e` | `version.json` read from the jar's central directory (no 34k-entry index); empty `datapacks/` reuses the shared datapack | 3.43 → 3.36 s |
| `987f7e3` | lowres flushes encode their PNGs on 4 scoped threads | 3.37 → 3.18 s |
| `7561353` | the next region's tile jobs and chunk area are prepared while the current one renders; tiles still recorded in plan order | 3.16 → 3.02 s |
| `4f2e578` | with `-v`, the local jar's packs load while the version manifest downloads | 3.05 → 2.98 s |

- **Why scoped threads, not rayon:** the PNG encode in `987f7e3` can't run on rayon, whose workers can be blocked
  sending to the persist thread. That deadlocked.
- **Why a state snapshot:** the prefetch in `7561353` means each render reads the block states as of loading its
  area (`RegionRender::registry`), because the shared registry grows while the next region loads.
- **Result:** rendering now starts at ~0.29 s and holds ~11–11.5 cores with no region-boundary dips. The tail is
  ~0.3 s at ~2 cores.
- **Cost:** peak RSS rises with two chunk areas held at once. Combined with the lowres-only commit, against
  `8bab4ae`: wall 3.28 → 2.63 s (median of the 3 quietest pairs), CPU −3%, peak RSS 379 → 413 MB.
- **Tried and dropped:**
  - final flush only on 12 threads: no gain, because the heavy flushes are the ones the last regions trigger
  - one rayon task per tile with pooled buffers: −4.5% CPU but +30–40 MB RSS
- **Left:**
  - startup ~0.29 s: client jar open ~60 ms, then textures, models and the gallery
  - tail ~0.3 s: the last region's slowest tiles, plus the serial lowres cascade (~95 ms), which could be computed
    in parallel and applied in order

## Round 6 (re-profiled master `26c46a5`)

Testbox, `perf` at 499 Hz (structures) and 99 Hz (the 4096² world of docs/14). Share of all samples:

| area | structures compat | real compat | structures optimized | real optimized |
|---|---|---|---|---|
| hires encode (gzip / BMQ2) | 39% | 47% | 34% | 44% (quads 22%, zstd 21%) |
| block pass | 28% | 30% | 31% | 32% |
| ↳ `fully_culled` | 8.6% | 8.5% | | 9.1% |
| `Volume::fill` | 5.9% | 5.6% | 6.3% | 5.9% |
| sort + `write_prbm` | 6.4% | 7.9% | 6.7% | 8.2% |
| chunk load | ~4–9% | 4.1% | ~9% | 6.3% |
| persist thread | 6.4% | 2.2% | 7.0% | 2.5% |

| change | effect |
|---|---|
| BMQ2 `pick_grid` read which grids hold a value exactly from the float's bits instead of three `quantize` calls per value | optimized CPU −5% structures, −7% real. Not landed: BMQ3 (docs/17) replaced the BMQ2 encoder |
| `Region::preload`: an area load reads each region's chunk sectors up front (runs within 256 KiB in one read), region files in parallel | Linux CPU neutral, webroot identical on structures and real |

- **Why preload:** minidump samples on Windows (`docs/perf-exp/pmp.py`) had 30% of the render threads inside
  `read_at_most`. Windows serialises I/O on a synchronous handle, and the render threads run at low priority, so a
  preempted reader holds up every other one. After the change no sample was in a read. The wall-clock gain on
  Windows is **unmeasured**: the PC ran at 19–20 of 22 cores busy from other work and 60 of 64 GB committed, and
  renders took 7–16 s either way. Re-measure on an idle machine (`tools/bench_render.py structures --only rs`).
- **Tried and dropped:**
  - PGO (`-Cprofile-generate` trained on structures compat + optimized): compat 21.1 s against 20.5 s, optimized
    18.05 s against 18.1 s. Nothing.
  - libdeflate level (structures, CPU / hires bytes): 1: 19.0 s / 116 MB, 2: 21.0 / 111, 3: 21.1 / 85.2,
    **4: 21.9 / 79.9**, 5: 24.0 / 82.3, 6: 27.9 / 78.6. Level 4 stays.
  - BMQ2 zstd level (CPU / map bytes): 3: 16.6 s / 16.5 MB, 5: 17.0 / 14.3, 7: 17.5 / 13.1, **9: 18.0 / 12.8**.
    Level 9 stays.
- **Left:** compat is bound by gzip, which only a format change removes. The Rust side is spread thin: the block
  walk with `fully_culled`, then one copy each for the material sort and `write_prbm` (writing the PRBM in sorted
  order straight from the unsorted model would save about one of them, ~3%). The optimized columns above and the
  zstd sweep describe BMQ2; BMQ3 (docs/17) has since replaced it and has not been profiled here.
