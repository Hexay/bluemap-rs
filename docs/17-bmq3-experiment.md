# 17 — BMQ3: the optimized hires encoding, and how it got there

Question: how far below BMQ2 (the first `optimized` encoding, docs/09) can a lossless hires encoding go?
Answer: to **0.40× of BMQ2** on real terrain at the same zstd level, encoding 1.4× faster and decoding about as
fast. First sized with Python scripts (`docs/storage-exp/exp3_*.py`), then built in Rust. BMQ3 is now the only
encoding of `bm_format::compact` (blob spec in its module docs); BMQ2 was removed before any release stored it.

Corpora:
- **real**: 1024 tiles = every 16th 16×16 bundle of the 4096² world's compat render (docs/14), 7.89 M quads.
- **fx**: every 2nd tile of the golden fixture webroots (`work/bluemap`), 6.68 M quads.

## Result
Each row adds one step to the row above. Size is the whole tile body as one zstd frame with the shipped level-9
parameters, relative to BMQ2; "vs gzip" is against the stored compat `.prbm.gz` (hires only).

| step | real | real vs gzip | fx | what it does |
|---|---|---|---|---|
| BMQ2 | 1.000 | 4.82× | 1.000 | 17.4 bits/quad on real |
| `attrs` | 0.951 | 5.07× | 0.953 | AO as 2 bits per vertex; light as one byte |
| `tpl_float` | 0.745 | 6.47× | 0.708 | quad = template id + block cell; no fixed point, no escapes |
| `tpl_hash` | 0.547 | 8.83× | 0.629 | random plant offsets derived from the block hash |
| `cols_rec` | 0.517 | 9.32× | 0.589 | cell as (column step, dy) bytes |
| `ao_geo` (= BMQ3) | 0.396 | 12.17× | 0.475 | AO predicted from the tile's own geometry |
| `light_map` (not shipped) | 0.379 | 12.74× | 0.454 | light predicted from cells earlier quads looked into |

- With zstd-19 instead of level 9 the last row is 0.89× of itself (0.33× of BMQ2-at-9); xz is 0.85×. The body is
  ~7× smaller than BMQ2's before compression, so a higher level costs less than it does today.
- `exp3_check.py` decodes the geometry model and compares bits: 0 unreproduced values on real (every 16th tile,
  407 k quads), 3 on fx (529 k quads), all three listed as exceptions by the encoder. AO, light and cells are
  residuals of predictions the decoder can repeat, so they are invertible by construction (the Rust decoder
  confirms it, below).

Where the 6.6 bits/quad of `light_map` go (real, streams compressed alone): cell 1.89, template id 1.31, light 0.85,
AO 0.84, color 0.29, shape tables 0.3, the rest 0.2.

## The steps
**Templates (`exp3_model.float_templates`).** The mesher forms every position as `fl32(L + B)`: L the block-local
f32 of the model vertex, B the integer block coordinate inside the tile (`bm-render/src/block_pass.rs`). So a quad is
(template, cell): per axis a table of 4-float shapes, a template = (x, y, z, uv) shape ids, and the decoder redoes the
f32 addition. This replaces the i16 residual records and both escape streams. A tile has a median of ~35 templates.
The encoder picks the cell freely (quad centre nudged against the normal) and verifies every value; a shape seen
first at a large B has lost fraction bits, so candidates are tried finest first.

**Hash offsets.** Grass, ferns and flowers are shifted by `(hashToFloat(x, z, seed) − 0.5) · 0.75` per block column
(`bm-render/src/resource/mod.rs`), 24 random bits per axis. Stored as data that is two new shapes per plant block
(24% of `tpl_float`'s bytes on real). Recomputing the hash in the decoder makes a plant quad (template, cell) like
any other: `fl32(fl32(shape + d) + B)`. It needs the tile's world block origin (tile × 32 + 2 on the default grid),
which the codec does not get today. 8.6% of real quads use it; 1.2% on fx.

**Cells.** Inside a material group the mesher's order survives the sort: columns in scan order, each top-down. Stored
as (column step ≥ 0, dy), dy against the top quad of the previous column when the column changes.

**AO (`exp3_ao.py`).** `testAo` counts occluding neighbours around a corner in front of the face. The decoder has no
blocks, so "occluding" becomes "the cell holds a full block face of a material flagged as occluder" (one bit per
material group, chosen by the encoder). Right for 96.8% of vertices on real, 98.4% on fx; the stream stores
(actual − predicted) mod 4 and drops from 2.41 to 0.80 bits/quad.

**Light (`exp3_light.py`).** Face light is max(own cell, cell in face direction). The decoder keeps the lowest light
seen per looked-into cell and predicts unseen cells by the propagation rule. Small gain (1.14 → 0.87 bits/quad) for a
stateful decoder; the first candidate to drop.

## Negative results
- **Context-modelling entropy coder instead of zstd** (`exp3_entropy.py`): adaptive order-1/2 contexts cost as much
  as or more than zstd-19 on every per-quad stream (AO 2.9–3.2 vs 2.4 bits/quad, ids 1.25 vs 1.31). The redundancy is
  long repeats, which LZ finds and low-order contexts don't.
- **Heights from earlier material groups** to predict dy: cell stream 1.88 → 2.20 bits/quad.
- **Grid templates with BMQ2's escapes** (`tpl_grid`): 1.04× on real. Templates only pay once they hold exact floats.
- **One interleaved record per quad** (id, cell, AO, light): worse than separate streams. (id, step, dy) alone
  (`rec3`): −3% at level 9, nothing at 19.
- Packing light into one byte alone changes nothing (`light4`).

## Rust implementation
Measured while BMQ2 still existed beside it: each stored tile encoded and decoded with both codecs on one thread,
output asserted equal to the input PRBM, fastest of 3 runs per tile summed. (BMQ2 is gone from the tree, so these
rows cannot be re-run; `examples/compact_bench.rs` reports BMQ3 alone.)

| corpus, zstd level | BMQ2 size | BMQ3 size | encode ns/quad (2 → 3) | decode ns/quad (2 → 3) |
|---|---|---|---|---|
| **real, all 1024 tiles, 9 (as shipped)** | 17.12 MB | 6.84 MB (0.399) | 672 → 465 | 150 → 142 |
| real, all 1024 tiles, 9, with the light predictor | 17.12 MB | 6.57 MB (0.384) | 713 → 507 | 172 → 187 |
| real, every 4th tile, 12, with the light predictor | 4.00 MB | 1.60 MB | 1356 → 909 | 164 → 186 |
| real, every 4th tile, 15, with the light predictor | 3.84 MB | 1.49 MB | 3029 → 1130 | 151 → 172 |

- **Exact:** every real and fixture tile round-trips; only empty tiles end up in raw mode. On the fixtures the
  storage oracle (`oracle_optimized`) measures 13.7× smaller than Java's gzip hires overall (BMQ2: 6.6×).
- **Encode is ~1.4× faster than BMQ2** at the same level: the model costs more than BMQ2's quantizer, but zstd sees
  a body ~7× smaller. Level 15 would take another ~8% off for 1.6× BMQ2-at-9's encode time; shipped is level 9.
- **Decode is on par** (~5% faster on real terrain), at ~1 GB/s of PRBM.
- **The light predictor is not shipped:** 4% of size for ~14% of decode time and a second per-tile grid; sparse
  tiles (the End fixtures) decoded 1.2–1.3× slower with it.
- The PC was shared with other builds, so absolute times move ±15% between runs; compare the two codecs within a
  row, not across rows.
- The occluder flags come from one voting pass (a vertex without occlusion clears the marked cells it looks at),
  not the Python search, and predict AO equally well (0.80 bits/quad).
- Block-local values carry rotation noise (5.96e-8 for 0, 0.99999994 for 1); the AO face test snaps within 1/4096.
- Shapes are found greedily in quad order instead of finest-first: template ids cost 1.41 instead of 1.31 bits/quad.

## How it is wired
- The tile's world origin: `MapStorage::set_hires_grid` (called by the engine's `MapContext` and by
  `--convert-storage` for configured maps) lets `OptimizedMapStorage` hand `CompactCodec::encode_into` the tile's
  minimum block. Without a grid tiles still encode, only offset plants cost more.
- A tile the model cannot hold (not a quad-shaped PRBM, no quads, ao or light values the mesher never writes,
  coordinates out of range) is stored raw: the PRBM under zstd.
- The storage marker is unchanged. A storage written by a build that still had BMQ2 fails per tile with "missing
  BMQ3 header"; re-render it.
- The predictors mirror BlueMap 5.28's mesher (hash formula, `testAo`). A different upstream version degrades size,
  never correctness: every prediction is checked by the encoder and has a verbatim fallback.

## Not done
- Finest-first shape discovery (~1.5%), template ids interleaved with cells (~3% at level 9), a higher zstd level.

## Caveats
- The real corpus is fresh vanilla terrain (plants, trees, no builds). The hash-offset step matters less on built-up
  maps: fx gains 11% from it, real 27%.
- Totals are hires only. Lowres PNGs are unchanged, so whole-storage ratios move less.
- The full real run was started before a fix to which table a shared shape row is read from (found by
  `exp3_check.py`); on every 8th tile the total is identical before and after (0.747 MB).

## Reproduce
```
py -3 -I docs/storage-exp/exp3_baseline.py real 1          # BMQ2 per stream + codec sweep
py -3 -I docs/storage-exp/exp3_stats.py real 16            # structure statistics
py -3 -I docs/storage-exp/exp3_variants.py real 1 baseline attrs tpl_float tpl_hash cols_rec ao_geo light_map
py -3 -I docs/storage-exp/exp3_variants.py real 32 sweep light_map   # codecs on the modelled body
py -3 -I docs/storage-exp/exp3_entropy.py real 8           # context-coder estimates
py -3 -I docs/storage-exp/exp3_check.py real 16            # bit-exact geometry decode
```
Rust (add `--streams` for the per-stream breakdown, `--level N`, `--step N`):
```
cargo run --release -p bm-format --example compact_bench -- work/exp/real --reps 3
cargo test --release -p bm-format --test compact_corpus -- --ignored --nocapture
```
The Python scripts model BMQ2 themselves (`bmq.py`), so their baseline rows still run.
`real` reads `work/exp/real` (copied from testbox `~/bmrs-real/work/real/rs`, tiles with `(x>>4)%4 == (z>>4)%4 == 0`);
`fx` reads `work/bluemap`. 4 workers (`exp3_lib.WORKERS`); the full real run takes ~8 min.
