# 20 — CPU profile of the optimized-storage render (BMQ3)

Round 7 of docs/12, the first one inside a `format: optimized` render since BMQ3 (docs/17) replaced BMQ2. Scope:
`bm-render`, `bm-engine` (not `persist.rs`), `bm-resources`; hotspots elsewhere are listed for their owners.

## Setup

- testbox (docs/14), engine `bluemap -c config -v 26.3 -r -f`, `--profile profiling`, 12 render threads.
- Fixtures: `real-opt` = the 4096² world of docs/14 with `format: optimized`; `structures-opt` = structures, optimized.
- Sampling (no lock): `docs/perf-exp/testbox_cpu.sh perf <fixture> <bin> <hz>` (every thread, dwarf stacks),
  `threads` mode for per-thread CPU from `/proc`. Ranking: `cpu_areas.py` (each sample to the deepest pipeline-area
  frame on its stack), `under.py` (what runs under a frame), `chain.py` (who calls a frame); `cpu_queries.sh` runs
  the drill-downs below.
- Off-CPU: `perf_event_paranoid=1` refuses `sched:*` tracepoints, so off-CPU time comes from the per-thread
  CPU/wall split instead (below).
- A/B: `testbox_cpu_ladder.sh <checkout> <fixture> <rounds> <ref>…` builds every ref and renders them all in one
  `~/bench.lock` per round, order rotated, with user CPU, `instructions:u` and `cycles:u`. The box was shared
  (load 20–120 during most runs), so user seconds swing ±3% run to run; instructions are near-deterministic and
  cycles track user time with less noise. Medians of 9 rounds (structures) or 3 (real).
- Exactness: `CONVERT=1` converts each optimized webroot to compat (`--convert-storage`) before diffing, since
  bundles hold records in completion order and differ byte-wise between any two runs of one binary.

## Before (real-opt, `15cf440` = master `871640d` + tooling, 99 Hz, 293.5 s user)

| area | share | notes |
|---|---:|---|
| BMQ3 encode (`CompactCodec::encode_into`, excl. zstd) | 25.3% | `encode::quads` 18%, `Shapes::build` 10% (structures) |
| zstd | 8.3% | inside the BMQ3 encode |
| model faces (`resource::render`) | 15.3% | face 1.2, AO 1.6, cullface/AO `state_at` 1.1, `transform_from` 1.1, `RelativeOffsets::get` 0.9, realloc 1.7 |
| `fully_culled` / `dark` | 11.4% | neighbour state + flags loads |
| `Volume::fill` | 6.9% | `Light::column` 1.2, `Padded::strided` ~2.7 |
| block walk | 6.6% | |
| `write_prbm` | 4.8% | floats 0.9, `unit_byte` 0.9, normals 1.0 |
| material sort | 4.1% | three gathers 2.2, the index sort ~1 |
| chunk load | 5.0% | inflate 2.4, parse 2.5 |
| lowres PNG (persist thread) | 2.8% | zlib-rs level 9 |
| tint/biome | 2.1% | |
| liquid | 1.5% | |
| resources load, startup | < 1% | |

Threads: render 96.4%, persist 3.3%, main 0.3%. Structures-opt is the same picture with more chunk load (9%) and
lowres PNG (5%): small tiles per chunk loaded.

Per-thread CPU (structures-opt, `threads` mode): render threads 16.8 s, persist 0.94 s, main 0.36 s over a 3 s
render: the render pool is saturated from start to tail, so CPU wins are wall wins as in docs/12.

## Landed

Each step against the previous one; structures-opt, 9 rounds, median.

| commit | change | user | cycles:u | instr:u |
|---|---|---:|---:|---:|
| `24c2d27` | `transform_from`: scale-only matrices (every unrotated element's `scale(1/16)`) skip the zero terms. Exact: the dropped terms are ±0, and adding the +0 translation normalises the zero's sign as the full product does | −0.5% | −0.7% | −0.1% |
| `c5e94fa` | rayon makes fresh `TileBuffers` per split; they now start with the last tile's face capacity, so the mesh doesn't regrow (each growth copies 7 arrays) | −1.8% | −1.6% | 0.0% |
| `6561520` | per variant, element and face: rotated facing y, cullface and light neighbour offsets and each corner's AO probes, resolved once in `StateCache` (`resource/faces.rs`) instead of per face | −1.2% | −1.3% | −1.9% |
| `83aadc8` | region headers read once per update (`HeaderCache`) instead of once per neighbouring region (9×) | 0 on a full render | | |

Structures-opt in total: 16.82 → 16.24 s user (−3.4%), cycles −3.6%. Real-opt, base against all of the above
(3 rounds): see "Result".

`HeaderCache`: a no-op `-r` of real-opt (100 regions) went 0.41 → 0.40 s, within noise; the gain is 8 of 9 region
opens and header reads per region, which scales with region count (a 300k² map has ~340k regions). A header stays
cached until the last region whose 3×3 neighbourhood holds it has loaded it, so memory is the spiral's frontier
(4 KB per region). It holds the timestamps only (no open file).

## Tried and dropped

| change | effect | why |
|---|---|---|
| `fully_culled` branch-free for full cubes (a `Flags::CUBE` bit, six neighbour tests without early exit) | structures cycles +5.1%, instr +2.0%; real user ±0, instr +1.9% | the early exits predict well; the extra loads cost more than the mispredicts saved |
| material sort on packed `(material, index)` u64 keys with `sort_unstable` | instr +1.25–1.6% | faces come in material runs, which the stable (merge) sort exploits |
| PRBM written straight from the unsorted model in material order (one pass reads each face once, writes all 7 attribute regions; no sorted copy) | instr +1.35%, cycles +3.2% (measured with the fixture accidentally on compat) | the per-attribute loops over contiguous arrays vectorise; one scattered pass over 7 outputs doesn't, and the gathers were cheap because runs are mostly sequential |
| `normal_bytes` (no sqrt for axis-aligned faces) in `write_prbm` | instr +0.8%, cycles +0.3% | its zero/range tests cost more than the sqrt and division they skip |
| per-thread retained `TileBuffers` (µarch/peak-RAM suggestion) | not rebuilt | docs/12 measured +30–40 MB RSS; `c5e94fa` gets the copy savings without retaining memory |

## Result

Real-opt, master `95b73fd` against `7ebfbf3` (all of the above), forced render, median: 293.6 → 280.3 s user
(−4.5%), instructions −3.5%, cycles −3.9%; peak RSS 895 → 916 MB (+2.4%, the per-split buffers starting at the last
tile's capacity). Tile output identical apart from rstate timestamps.

## Remaining hotspots (ranked, after)

No after-profile was taken; the ranking below ("Outside these crates") is from the before-profile less the steps
above.

## Outside these crates (for their owners)

- **BMQ3 encode + zstd, ~34% of an optimized render** (bm-format compact): the largest single area.
  `encode::quads` and `Shapes::build` dominate; the encoder re-parses the PRBM bytes the render just wrote
  (`PrbmView`). An encoder that takes the sorted `TileModel` (or the unsorted one plus the order) would drop the
  write and the re-parse for optimized storage; `stored_hires_equals` would then compare models instead of bytes.
- **Lowres PNG, 2.8% real / 5% structures** (persist thread, zlib-rs level 9 via `png`): pinned by byte identity.
- **`Volume::fill` 6.9%** is mostly `bm_world::Chunk::column_into` (`Padded::strided`, `Light::column`). Proposed
  bm-world API so the volume is filled in section storage order (y, z, x; one packed word decoded once for 16 x):
  `Chunk::fill_box(&self, x: Range<i32>, y: Range<i32>, z: Range<i32>, dst: BoxDst)` where
  `BoxDst { states: &mut [StateId], light: &mut [[u8; 2]], origin: usize, step: [isize; 3] }` — the chunk writes its
  part of the box to `origin + x·step[0] + y·step[1] + z·step[2]`, uniform sections and light as strided fills.
  `Volume::fill` would then loop over the ≤ 9 chunks under the tile with steps `[d·h, 1, h]` (its x-major,
  y-innermost layout, which the column walk wants) and apply `Masking::edge_column` afterwards, as now.
- **Chunk load 5%** (inflate 2.4%, NBT parse 2.5%): each region's area loads a one-chunk ring of its neighbours'
  chunks again (34² of 32² chunks, ~13% of the parse).
