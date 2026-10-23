# 19 — Optimized storage size, round 2

Question: what else takes lossless bytes off an `optimized` file storage, for a 300,000² map (~88 M hires tiles,
~700 GiB at BMQ3 as of docs/17)? Answer: an encoder-only change (finest-first shapes, landed, −3%) and a higher zstd
level (−10% at 19, which makes the whole tile encode 6× slower; not landed). Cross-tile context, stream re-layouts and a 2D cell
predictor buy ≤1.6%. Lowres is ~7% of the total; WebP lossless would take a third of that off.

Corpora as docs/17: **real** = `work/exp/real` (1024 tiles, 4 whole bundles of the 4096² vanilla world, 7.89 M
quads), **fx** = every 2nd tile of the golden fixtures (`work/bluemap`, 1019 tiles, 6.63 M quads), the stand-in for
built-up maps.

## Where the bytes go
The 4096² world rendered `optimized` (testbox `~/bmrs-real/work/real/rs-bmq3`, BMQ3 before this round):

| part | files | bytes | share |
|---|---:|---:|---:|
| hires bundles | 81 | 127.5 MB | 91.1% |
| lowres lod 1/2/3 PNG | 144/16/4 | 10.2/1.4/0.1 MB | 8.4% |
| rstate, textures, settings | | 0.6 MB | 0.5% |

- 16,641 hires tiles at 7.66 KB each → 88 M tiles ≈ 674 GB. The 4 KiB block slack of bundles is 0.13% and record
  plus blob headers 21 B per tile (0.27%): container overhead is not worth touching.
- The lod 1 grid overhangs the rendered area; the 64 fully covered lod 1 tiles average 144 KB. Scaled by area
  (360 k lod 1 tiles + lod 2/3) lowres is ~54 GB at 300,000², ~7% of the storage.

## Results
Size is the zstd frame of every tile's model body, relative to the shipped encoder at level 9 (before this round).
`compact_exp` measures layouts and prefixes on the bodies the encoder writes. Times: testbox (6-core Xeon
E-2136, under `~/bench.lock`), `--release`, single thread, every 16th tile of the 4096² world (1041 tiles, 10.2 M
quads; `docs/storage-exp/r2_bench.sh`). Whole-tile BMQ3 encode is 370 ns/quad at level 9, decode 90 ns/quad; the
zstd columns are zstd alone on the same bodies (level 9: compress 84, decompress 5.6 ns/quad).

| idea | real | fx | encode (ns/quad) | decode (ns/quad) | verdict |
|---|---:|---:|---|---|---|
| **finest-first shape discovery** | **−3.1%** | **−2.9%** | 362 → 371 (+2.5%) | 95 → 90 | **landed** |
| zstd 12 | −2.0% | −1.3% | zstd 84 → 292 | ±0 | no |
| zstd 15 | −8.8% | −7.8% | zstd → 552 (tile encode ×2.3) | ±0 | option, not default |
| zstd 17 | −9.6% | −8.5% | zstd → 1,046 (×3.6) | +0.6 | option |
| zstd 19 | −10.1% | −9.3% | zstd → 2,067 (×6.3) | +0.2 | option |
| zstd 22 | −10.3% | −9.4% | ~2.3× of 19 (4-thread wall) | | no |
| finest-first + zstd 19 | −12.6% | −11.5% | | | best total |
| a zstd block per stream (flush), 9 / 19 | −0.4% / +3.0% (vs z19) | +1.0% / +4.4% | | | rejected |
| ids interleaved with cells (`id, step, dy`), 9 / 19 | −0.7% / +0.8% | −0.9% / +0.1% | | | rejected |
| cells as step plane + dy plane, 9 / 19 | +3.5% / +2.7% | +4.1% / +3.1% | | | rejected |
| run top from a 2D (MED) prediction of the same group's columns, 9 / 19 | −0.2% / −0.3% | 0 / −0.1% | | | rejected |
| prefix = left neighbour's body (unbounded chain), 9 | −1.6% | −0.4% | | needs all left tiles | rejected |
| prefix = neighbour, checkerboard (one hop), 19 | −1.3% vs z19 | −0.3% vs z19 | | +1 frame decompress | rejected |
| prefix = bundle key tile (8, 8), 19 | −1.2% vs z19 | −0.8% vs z19 | | +1 frame decompress | rejected |

- The two leftovers of docs/17 measured smaller than estimated: interleaving was "~3% at 9" in the Python model,
  here −0.7% and a loss at 19.
- **Cross-tile context is small.** Even an unbounded chain of left neighbours as zstd prefix gives 1.6% on real and
  0.4% on fx; the one-hop schemes ~1%. The bodies have little in common across tiles once the model has removed
  geometry; it is not worth the client-side unpacking (docs/18: the browser would need the prefix tile's body too).
- zstd decompression is ~6% of a tile's decode and does not get slower at higher levels (5–6 ns/quad at 9–19), so
  the level costs the browser nothing.

### Finest-first shapes (landed)
A shape is the block-local value of one axis of a quad's 4 vertices; the decoder computes `fl32(shape + B)`. A
value derived from a quad at a large B has lost fraction bits, so it may not reproduce the same model vertex at a
small B, and the greedy encoder then added a second shape (and template) for it. The encoder now makes a first
pass over the quads that keeps, per shape key, the value seen at the smallest |B| and seeds the candidate lists
with it; values that still do not fit go behind it (`compact/shapes.rs`).

- Format and decoder unchanged: old blobs decode as before, `bm-wasm`/`unpack.wasm` need no rebuild.
- Template ids 1.41 → 1.30 bits/quad, templates table 0.23 → 0.20, shape tables −0.02 (real, zstd 19 per stream).
- real 6.836 → 6.627 MB at level 9 (blob totals, `compact_bench`); the storage oracle (`oracle_optimized`, Java's
  fixtures converted, every tile read back exactly) is 14.1× smaller than Java's gzip hires, from 13.7×;
  `compact_corpus` round-trips all 2085 fixture tiles.
- Testbox, same 1041 tiles: 8.076 → 7.839 MB (−2.9%); encode 362 → 371 ns/quad (+2.5%, the extra pass); decode
  95 → 90 ns/quad (fewer templates; the baseline's second run was noisier, 134). Blob hashes are stable across runs.

### zstd level
At ~9,800 quads per tile, level 15 adds ~4.6 ms and level 19 ~19 ms of zstd per tile, against ~28 ms CPU per tile
for the whole render (docs/14: 471.6 CPU s / 16,641 tiles). Level 19 would add ~70% render CPU for 10% of storage. Not a default; worth it as a deliberate pass for a map that is mostly static: the level
is not part of the format (any level decodes), so a later recompress of existing bundles (e.g. at compaction or a
`--convert-storage` re-encode) needs no format change. Not built.

## Lowres
Lowres tiles are written by `bm_format::lowres::LowresTile::encode_png` (png crate, no filter, deflate 9), so the
pixels determine the stored bytes; a storage could keep a smaller lossless form and re-encode the identical PNG.

| lowres (PNG = 1) | real lod 1 (144) | real lod 2 | real lod 3 | fx lod 1 (276, Java PNGs) |
|---|---:|---:|---:|---:|
| WebP lossless (`exact`, method 6) | 0.664 | 0.701 | 0.601 | 0.263 |
| xz -9e of raw RGBA | 0.677 | 0.749 | 0.680 | 0.347 |
| zstd 19 of raw RGBA | 0.806 | | | |
| zstd 19 of channel planes / left / up / MED residuals | 1.22 / 1.30 / 1.41 / 1.38 | | | |

- All WebP files decode to the identical pixels.
- At scale WebP saves ~⅓ of ~7% ≈ 2.3% of the storage (~16 GB at 300,000²), zstd of raw pixels ~1.4%. Both need a
  PNG encode per served tile (or a cache, or a client unpacker like docs/18 that would also have to cover the
  webapp's `RevalidatingFileLoader`), WebP also a C library. Not built: smaller than the zstd-level lever and with
  a serving cost compat does not have.
- The fixture lowres is mostly empty margin (small worlds), hence its low ratios; its lowres share (2.2 MB against
  9.4 MB of BMQ3 hires) does not scale with area.

## Not done
- A recompress pass at a high zstd level for static regions (above).
- Light predictor stays rejected (docs/17); no new stream lever above 1% turned up besides the shape seeding.

## Reproduce
```
cargo run --release -p bm-format --example compact_bench -- work/exp/real --reps 3 [--streams]
cargo run --release -p bm-format --example compact_exp -- work/exp/real [--only z9,z19,…] [--time]
cargo run --release -p bm-format --example compact_exp -- work/bluemap --step 2
# from Git Bash, MSYS_NO_PATHCONV=1 keeps /home/… arguments as given
py -3 tools/testbox.py run r2 --sync --lock -- bash docs/storage-exp/r2_bench.sh <tiles dir> <baseline shapes.rs>
py -3 -I docs/storage-exp/lowres_r2.py <dir with tiles/1…>        # WebP / xz
py -3 -I docs/storage-exp/lowres_r2_zstd.py <tiles dir>            # zstd on pixels, lod 1
```
