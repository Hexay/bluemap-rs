# 09 — Hires storage-format experiment (PRBM vs compact lossless encoding)

Question: how much smaller can hires tiles be stored losslessly than BlueMap's gzip'd PRBM?
Data: real BlueMap 5.27 output, `bluemap_reverse/work/bluemap/*/web/maps/*/tiles/0` (2996 tiles, 14 maps).
Scripts: `docs/storage-exp/` (run with `py -3 -I <script>`; see "Reproduce").

## TL;DR
- **Zero format change:** serving or storing the same PRBM bytes as zstd-19 or brotli-11 gives **0.63–0.65×** stored gzip.
  A trained zstd dictionary adds nothing because tiles are ~1.2 MB raw.
- **Compact quad encoding (BMQ1) + zstd-19:** **0.133× stored gzip (7.5× smaller)**. With brotli-11 it is 0.129×.
  Raw BMQ1 is 3.0× smaller than raw PRBM. It is **byte-exact lossless**: 500/500 sampled tiles decode to
  identical PRBM bytes. An earlier pass over 50 tiles with the same code shape also passed.
- The redundancy is large: every triangle pair is a quad, and normals are fully derivable from positions
  (100% of triangles). Color and light are constant per quad (100%). About 91% of position values sit on a
  1/16 grid and 99.7% of UV values do.
- Dedup saves only 3.9%, all of it cross-map (identical seeds rendered twice). There are no within-map
  duplicates apart from 46 empty tiles.
- 4 KiB cluster overhead is +2.9% for hires and +18% for lowres PNGs.

## Method
- `prbm.py`: numpy PRBM parser and writer. `write(parse(x)) == x` holds for **2996/2996** tiles, so the
  writer is canonical and a lossless codec only has to reproduce the attributes and groups.
- `analyze.py`: baseline sizes and redundancy statistics over **all** tiles.
- `dedup.py`: SHA-256 of the raw PRBM. `lowres.py`: all 445 lowres PNGs.
- `compact.py`: encodes BMQ1, checks `decode(encode(x)) == x` byte for byte, times the transcode, then
  measures codecs for PRBM and BMQ1.
  - It ran on a **1/6 sample (500 tiles, every 6th sorted path, 34.7 MB stored = 16.5% of corpus)**. A full
    run takes about 1 h in Python because brotli-11 costs ~8 µs/byte on 3.6 GB of PRBM.
  - Brotli-11 was measured on every 4th sampled tile (125 tiles).
  - The zstd dictionary was trained on 150 random tiles from the even half and tested on the odd half.
- Codecs: Python `gzip` 6/9 (mtime=0), `zstandard` 0.25 level 19, `brotli` quality 11.
  BlueMap's stored files equal gzip-6 to within 0.1%.

## 1. Baseline (all 2996 tiles)
| map | tiles | stored gz MB | 4K-cluster MB | raw PRBM MB | raw/gz | verts M | quads M | gz B/quad |
|---|---|---|---|---|---|---|---|---|
| structures | 1153 | 81.01 | 83.34 | 1433.0 | 17.7 | 49.40 | 8.23 | 9.8 |
| structures_numeric | 911 | 78.11 | 79.97 | 1328.9 | 17.0 | 45.81 | 7.64 | 10.2 |
| vanilla_512 | 289 | 19.90 | 20.49 | 350.5 | 17.6 | 12.08 | 2.01 | 9.9 |
| nether | 81 | 7.95 | 8.11 | 124.9 | 15.7 | 4.31 | 0.72 | 11.1 |
| debug | 169 | 6.15 | 6.52 | 98.9 | 16.1 | 3.41 | 0.57 | 10.8 |
| vanilla / vanilla_edited | 81+81 | 4.69+4.70 | 4.87+4.86 | 83.0+83.0 | 17.7 | 2.86×2 | 0.48×2 | 9.8 |
| 7 small maps | 231 | 7.24 | 7.80 | 136.4 | ~16.5 | 4.69 | 0.78 | ~10.3 |
| **total** | 2996 | **210.8** | **217.0 (+2.9%)** | **3638** | 17.3 | 125.4 | 20.9 | 10.1 |

- Raw bytes per attribute: position 41.4%, uv 27.6%, normal 10.3%, color 10.3%, ao 3.4%, blocklight 3.4%,
  sunlight 3.4%. The header and groups are under 0.1%. Each vertex costs 29 B, so each quad costs 174 B.
- 46 tiles are empty, with 0 vertices and 100 B raw.

## 2. Redundancy (all tiles)
| property | result |
|---|---|
| triangle pairs are quads `(v0,v1,v2),(v0,v2,v3)` (pos+uv+ao identical) | **100%** of non-empty tiles (2950/2950); every group count is a multiple of 6 |
| unique vertices (all 7 attrs) / total | 64.5% (indexing alone saves ~1/3 of the vertex payload) |
| unique positions / total | 17.5% |
| unique pos+uv+ao / total | 52.4% (≈ the 4-of-6 quad vertices; indexing past quads gains little) |
| normal constant per triangle / per quad | 100% / 99.8% (non-planar liquid quads) |
| normal **recomputable** from positions (PRBMWriter formula in f64) | **100%** of triangles |
| color, blocklight, sunlight constant per triangle and per quad | 100% / 100% |
| ao constant per triangle / per quad | 67.9% / 66.7% (genuinely per vertex) |
| distinct normals (corpus) | 1089 (a u16 palette would work; derivation makes it unnecessary) |
| positions exactly on a 1/16 / 1/32 / 1/256 grid (f32 round-trip) | 90.5% / 90.9% / 91.3%; no -0.0 |
| position range (tile-local) | -64 .. 224.125; per-tile span ≤ 3586 at 1/16 (12 bit) and ≤ 57376 at 1/256, so **u16 fixed-point works for every on-grid value** |
| UVs on a 1/16 / 1/32 / 1/256 grid | 99.7% / 99.8% / 99.9%; range -7.7e-7 .. 1.0000007 (rotation float noise) |
| quads that are axis-aligned 1×1 block faces with UVs in {0,1} | 48.6% overall (structures 37%, structures_numeric 63%, nether 84%) |

Off-grid positions, from `offgrid.py` on a 1/60 sample (8.2% of values; almost all on the X/Z axes):
- Most have fractional part .05 or .95. These are cross models (kelp, short_grass, fern, wildflowers) with
  0.8/16 insets, and the f32 sum `block + 0.05f` is not k/2^n.
- The rest are random block offsets (grass, flowers) and rotated elements. They can't all be eliminated, but
  they are near a grid value.

## 3. Recompressing unchanged PRBM (1/6 sample, ratio vs stored gzip)
| codec | ratio | note |
|---|---|---|
| gzip -9 | 0.99 | no gain over BlueMap's default level |
| **zstd -19** | **0.63** | all maps 0.59–0.69 |
| zstd -19 + dict (16 KiB / 9 KiB trained) | 0.65 / 0.65 (test half; plain zstd-19 on the same half: 0.63) | dicts don't help with 1.2 MB samples. Trainer returned 9 KiB when asked for 112 KiB |
| brotli -11 | 0.65 (zstd-19 on the same subset: 0.63) | ~8 µs/byte to compress, so not viable at render time |

Browsers accept `Content-Encoding: br` everywhere and `zstd` in Chromium/Firefox. The webapp needs no change,
but the server must negotiate the encoding, keeping gzip as the fallback.

## 4. BMQ1 compact encoding (`storage-exp/bmq.py`)
Design. All streams are length-prefixed. Every predictor is exactly invertible, so decode is exact.
- **Geometry:**
  - One record per quad, taken from PRBM vertices 6q+{0,1,2,5}. Triangle 2 is implied.
  - Pos and UV are i16 fixed point at a per-tile grid of 2^g, g ∈ {4,5,8}, picked to minimise escapes. 78%
    of tiles use (pos 1/16, uv 1/16).
  - Prediction is parallelogram within the quad (`v1-v0, v2-v1, v3-(v0+v2-v1)`), with v0 delta-coded
    against the previous quad.
  - Stored AoS, one 24 B record per quad. AoS beat SoA byte planes by 40% under zstd because LZ matches
    repeated quads whole.
- **Escapes:** off-grid values, as a gap-coded index plus an f32 *bit* residual against the nearest grid
  value, in 4 byte planes. This is exact for any finite float, including -0.0, and cut escape bytes 5×
  compared with storing raw f32. NaN/inf are untested; the corpus has none.
- **Normals:** not stored. The decoder recomputes them with the PRBMWriter formula and applies an exception
  list of verbatim quads (~0 entries).
- **Per-quad attributes:** color (3 B, SoA) and blocklight/sunlight (2 B). Any quad whose 6 vertices differ
  is stored verbatim (0 occurrences).
- **AO and groups:** ao is 4 B per quad, AoS. Groups are (material u32, quad count u32), with starts derived.
- **Fallback:** empty or non-quad tiles store raw PRBM (6/500 in the sample, all of them empty).

Results (1/6 sample, ratio vs stored gzip; raw PRBM is 17.3× stored):
| map | BMQ raw | BMQ gz9 | **BMQ zstd19** | stored/BMQ-zstd |
|---|---|---|---|---|
| structures | 5.92 | 0.16 | 0.121 | 8.3× |
| structures_numeric | 5.80 | 0.19 | 0.145 | 6.9× |
| vanilla_512 | 5.81 | 0.14 | 0.111 | 9.0× |
| nether | 4.58 | 0.22 | 0.185 | 5.4× |
| debug | 5.21 | 0.19 | 0.149 | 6.7× |
| vanilla / vanilla_edited | 6.04 / 5.76 | 0.14 / 0.15 | 0.107 / 0.118 | 9.4× / 8.5× |
| others (7 maps) | 4.5–5.5 | 0.12–0.23 | 0.098–0.195 | 5.1–10.2× |
| **total** | **5.76** | **0.17** | **0.133** | **7.5×** |

- Other codecs on BMQ1: brotli-11 0.129 (zstd-19 on the same subset: 0.140). zstd-19 with a dictionary
  scores 0.131 / 0.132 on the test half (16 KiB / 31 KiB dict), against 0.132 without one.
- Where BMQ1+zstd19 bytes go (`streams.py`, 1/10 sample): positions 44%, ao 22%, position escapes 19%,
  light 8%, uv 4%, color 1.5%, groups 1%, normal/color/light exceptions <0.5%.
- **Verifier:** 500/500 byte-exact PRBM round-trips (`decode(encode(x)) == x`, stronger than
  attribute-equal), with 0 failures.
- Projected to the whole corpus: 210.8 MB → ~28 MB with BMQ1+zstd, versus ~133 MB with PRBM+zstd.

## 5. Dedup (all tiles)
- 2797 unique hashes out of 2996 tiles; 127 duplicate groups covering 199 redundant tiles.
- Saving: 8.15 MB of gz, 3.9%.
- All non-empty duplicates are cross-map: structures↔vanilla_512 (36 groups), end↔dimensions/end (23),
  nether↔dimensions/nether (15), vanilla↔vanilla_edited (13).
- Within a map, the only duplicates are the 46 empty tiles. Content-addressed storage is worth it only for
  multi-map setups of the same world, or to share one empty-tile blob.

## 6. Lowres PNG (all 445, RGBA; out of scope for the webapp, sizes only)
| lod | n | PNG kB | 4K-cluster kB | PIL PNG optimize kB | WebP lossless (exact) kB |
|---|---|---|---|---|---|
| 1 | 333 | 2671 | 3154 | 2460 (0.92) | 872 (0.33) |
| 2 | 56 | 458 | 541 | 383 (0.84) | 175 (0.38) |
| 3 | 56 | 167 | 262 | 149 (0.89) | 32 (0.19) |

- All re-encodings are pixel-identical; `exact=True` is needed to keep RGB under alpha 0.
- Lowres is 1.5% of hires bytes, so it is not where the storage goes. oxipng/cwebp weren't installed;
  PIL's encoders were used instead.

## 7. Transcode cost (BMQ1 → PRBM → gzip, per tile, avg 1.2 MB raw PRBM)
- Python/numpy, measured under 8 parallel workers: encode 87 ms, decode to PRBM bytes 53 ms, gzip-6 of the
  result 124 ms. An earlier uncontended 50-tile run measured 55 / 26 / 58 ms.
- **Estimate, not measured:** Rust would take ~1–5 ms to decode (10–50× faster). Gzip-6 at ~30–60 MB/s would
  then dominate at ~20–40 ms per tile. Serving zstd/br directly, or decoding BMQ1 in the webapp, avoids that
  gzip step.

## Caveats
- The fixture worlds are small and partly synthetic (structure grids, debug, superflat). Real survival maps
  have more terrain unit faces and fewer cross models, so ratios will shift. The biggest maps here are
  structure showcases.
- Codec and BMQ numbers come from a 1/6 deterministic sample (brotli from a 1/24 sample). Baseline, redundancy,
  dedup and lowres numbers cover the full corpus.
- BMQ1 is a prototype. The obvious next steps are an AO palette with per-quad-constant flags (ao is 22% of
  the output) and a cross-model/offset-aware position predictor (escapes are 19%). Typed-array decoding in the
  JS loader is not yet written.
- Unit-face-only encoding (48.6% of quads) was measured but not implemented. BMQ1's parallelogram plus LZ
  already captures most of that redundancy.

## Reproduce
```
py -3 -I docs/storage-exp/analyze.py        # §1-2 -> out/analyze.json (~4 min, 8 workers)
py -3 -I docs/storage-exp/dedup.py          # §5
py -3 -I docs/storage-exp/compact.py 6      # §3-4,7 on 1/6 sample (~13 min); 1 = full (~1 h)
py -3 -I docs/storage-exp/report.py analysis | compact 6
py -3 -I docs/storage-exp/lowres.py         # §6 (~5 min)
py -3 -I docs/storage-exp/streams.py 10     # per-stream BMQ1 breakdown
```
`codec_sizes.WORKERS` is 8, sized for ~2 GB of free RAM.
