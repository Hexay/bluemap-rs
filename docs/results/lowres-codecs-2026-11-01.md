# Lowres tile codecs: how much smaller losslessly (devbox, 2026-11-01)

Offline experiment, no product change. Regenerate: `docs/perf-exp/lowres_codecs/run.sh <checkout> sizes-d12|sizes-k8|extra-d12|extra-k8|timed|browser`,
then `agg.py`, `savings.py`.

**Answer: lossless WebP at z9 reaches 0.67–0.69× PNG, but costs 2.4 s/tile against 51 ms today.** Better PNG reaches
0.91×. A custom codec reaches 0.71–0.73× and is the only option that encodes faster than today at a useful size, but
it needs a script. Delivered gain is 0.5 s at 50 Mbit/s on the 4096² first load. On a much bigger map it is an
estimated 3–4 s of 15 s, because LOD 2 and 3 dominate there.

## What a lowres PNG holds

Sources: `crates/bm-format/src/lowres.rs`, and the shaders in the bundled webapp (`LOWRES_FRAGMENT_SHADER`, `LOWRES_VERTEX_SHADER`).

- 8-bit RGBA, non-interlaced, `(size+1) × 2(size+1)` = 501×1002. Ours: unfiltered, zlib 9. The extra row and column
  duplicate the neighbour tile.
- Top half: straight ARGB colour. Alpha is used: 0 = void, `mix(void, rgb, a)`; partial alpha is rare (0.01–0.8% of
  pixels on these tiles).
- Bottom half: R = block light, G/B = 16-bit height (`g*65280 + b*255`), A = 255, or 0 if the column was never written.

**Your memory is correct.** Height and light share the PNG with colour. What lossy would break:

- Height drives vertex position and the shade term (`heightDiff * 0.06`), so an error of 1 moves geometry and shading.
- Light is read as `r*255`; it matters when `sunlightStrength` is low.

Could tolerate loss (not pursued):

- Colour RGB could take small errors.
- The shader never reads meta alpha, or colour RGB under alpha 0 (`mix` gives void). Fixtures hold RGB = 0 there anyway:
  0 exceptions on all sampled tiles.

## Candidates

Ratio is against the stored PNG bytes. **4096²**: all 164 tiles, 11.70 MB, except where noted. **k=8**: 96/64/16 tiles
per LOD, size-stratified, extrapolated by LOD to the map's 712.8 MB. k=8 repeats terrain, so it is not representative
of a real map of that size, though each tile holds real terrain.

Encode and decode are ms/tile on one thread, k=8 sample, under `~/bench.lock`: 24/16/8 tiles, or 3/3/3 for rows marked †,
so † timings are rough. Decode is native; browser decode is below.

| candidate | L1 | L2 | L3 | total 4096² | total k=8 | enc | dec | delivery |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| today (`encode_png`) | 1.000 | 1.000 | 1.000 | 1.000 | 1.000 | 51 | 1.0 | any |
| oxipng o2 | 0.902 | 0.937 | 0.872 | 0.906 | 0.918 | 257 | 1.2 | same URLs |
| oxipng o6 (subset‡) | 0.906 | 0.933 | 0.864 | 0.909 | 0.909 | 3324† | 1.2 | same URLs |
| oxipng o4 + zopfli, 5 iterations (subset‡) | 0.899 | 0.938 | 0.853 | 0.903 | 0.906 | 5969† | 1.2 | same URLs |
| WebP z3 exact | 0.947 | 0.718 | 0.615 | 0.917 | 0.948 | 120 | 3.1 | server swap |
| WebP z6 exact | 0.953 | 0.718 | 0.613 | 0.922 | 0.951 | 147 | 3.0 | server swap |
| WebP z8 exact | 0.939 | 0.706 | 0.606 | 0.908 | 0.937 | 312 | 3.0 | server swap |
| **WebP z9 exact** | **0.664** | 0.701 | 0.601 | **0.668** | **0.689** | 2404† | 2.6 | server swap |
| zstd 9, RGBA scanlines | 0.989 | 0.958 | 0.849 | 0.984 | 1.008 | 10 | 0.5 | script |
| zstd 19, RGBA scanlines | 0.806 | 0.883 | 0.799 | 0.815 | 0.833 | 182† | 0.6 | script |
| zstd 9, planes, no prediction | 1.365 | 0.981 | 0.788 | 1.314 | 1.348 | 8 | 0.8 | script |
| zstd 9, planes predicted per plane | 1.276 | 0.852 | 0.700 | 1.221 | 1.251 | 24 | 3.7 | script |
| zstd 19, pixels + predicted height | 0.744 | 0.833 | 0.752 | 0.755 | 0.770 | 126† | 1.6 | script |
| zstd 9, palette + predicted height | 0.782 | 0.900 | 0.917 | 0.797 | 0.815 | 12 | 1.4 | script |
| zstd 19, palette + predicted height | 0.701 | 0.795 | 0.816 | 0.713 | 0.728 | 89† | 1.7 | script |

‡ 8/8/4 tiles (4096²) and 6/6/6 (k=8), size-stratified.

Reference only, 18 LOD 1 tiles (4096²) and 8 LOD 2 tiles (k=8), not assessed for browser support:

- JXL e7 lossless: 0.78 and 0.72, identical pixels, 108 and 296 ms/tile (Pillow, not under the lock).
- AVIF at Pillow `quality=100`: 1.16 and 0.74, **not identical** on 24 of 26 tiles. No true-lossless AVIF was tested.

WASM decode, estimated: about 1.9× native (BMQ3 in Chrome vs native, docs/24, different code), so ~3 ms/tile for
palette, plus wrapping the pixels for `<img>` (below).

Why prediction loses: colours repeat exactly (858–1668 distinct per LOD 1 tile, ~27k on dense LOD 2/3). LZ on whole
pixels exploits that, and splitting or predicting destroys it. This is the same effect that makes unfiltered PNG beat
filtered PNG. Height is ~20% of the predicted form's bytes on LOD 1 and light ~0.6%.

## Verified identical

- **Native:** every candidate's decoder output was compared with the source RGBA on every tile it encoded. A mismatch
  panics; none occurred. The source includes RGB under alpha 0, which is always 0 on real tiles.
- **Chrome 1248 headless, WebGL2 readback** (`browser_check.mjs`; 9 real tiles + 1 synthetic with non-zero RGB under
  alpha 0 and every partial alpha). Firefox and Safari were not tested. The stated pixel-store settings are three.js's.

| file | `<img>` (first load) | `createImageBitmap` (after `clearTileCache`) |
|---|---|---|
| source PNG, oxipng, stored PNG | identical | **alters partial-alpha pixels; synthetic RGB under alpha 0 lost** |
| WebP exact | identical, synthetic included | the same kinds of change, on more pixels (e.g. 75 vs 1, 79 vs 48) |
| WebP without `exact` | **RGB under alpha 0 changes** (up to 244 k pixels) | as above |

- The `createImageBitmap` change already happens with today's PNGs, in upstream's own code path: it omits
  `premultiplyAlpha:"none"`. No opaque pixel differed on any path, so height and light are safe.
- On real tiles WebP and PNG differ from each other on that path, only at partial alpha. Visible effect not measured;
  it is weighted by that alpha in the shader.
- Non-`exact` WebP would be invisible per the shader, but it is not pixel-identical and saves almost nothing.
- Browser decode via `createImageBitmap`, single runs: PNG 2–5 ms, stored PNG 3–10 ms, WebP z6 2–8 ms.

## Delivery (code reading)

- **Two load paths** (`RevalidatingTextureLoader`). First load: `ImageLoader`, an `<img>`, which does **not** go through
  `fetch`. After `clearTileCache` sets `revalidatedUrls`: `fetch` → blob → `createImageBitmap`.
- **Script:** `unpack.js` wraps `fetch` only, so it misses first-load lowres. Intercepting without patching the webapp
  needs a hook on `HTMLImageElement.src` (blob URL of a PNG rebuilt from decoded pixels). Feasible on paper, not built.
  A stored PNG is 2 MB per tile per view, on top of the decode.
- **Colour:** the loader sets no colour space, and three.js uses `UNPACK_COLORSPACE_CONVERSION_WEBGL = NONE` for
  `NoColorSpace`, `UNPACK_PREMULTIPLY_ALPHA = false`. Nearest filtering, so no bleed from neighbours.

| candidate | how | catch |
|---|---|---|
| Better PNG | no script, same URLs, any storage | Encode cost. Check Java re-reading a colour-reduced tile (ImageIO, see the gray-sample gotcha); not tested |
| WebP | server swap on `Accept: image/webp`, script not needed for `<img>` | `fetch` sends no such `Accept` (from memory: verify), so the second path and non-browser clients need PNG: store both, or transcode at ~51 ms. Needs `Vary: Accept` (`map_handler` already does this for hires) and a per-form ETag. `optimized` only |
| Custom | script only, plus `server-unpack` for everyone else | `src` hook, a second decoder in WASM, PNG transcode for non-script clients |

**Drop-in and oracles:** oxipng can apply to `compat`. The lowres oracles found compare **pixels**, not bytes
(`bm-golden` `compare/tiles.rs`, `bm-map/tests/lowres_golden.rs`; `lowres.rs` says "the PNG bytes may differ"). A grep of
`accept_optimized.py` and `bm-storage/src/format.rs` finds no lowres byte check. So byte-identity between `optimized` and
`compat` lowres looks like documented behaviour rather than something these oracles enforce; the search was not
exhaustive. Docs stating it would need changing.

## What it buys (calculated, bandwidth only)

**4096² first load:** `saved × 8 / rate`, on the 92 lowres tiles of the 2026-11-01 breakdown, stored PNG 10022 KB of
11043 KB. All-requests-done at bandwidth alone: 1.77 s at 50 Mbit/s, 8.83 s at 10. Latency and the 8-in-flight limit are
ignored. Script decode and the blob round trip are not subtracted.

| candidate | view ratio | saved KB | load KB | s saved at 50 Mbit/s | s saved at 10 Mbit/s |
|---|---:|---:|---:|---:|---:|
| oxipng o2 | 0.916 | 841 | 10202 | 0.13 | 0.67 |
| WebP z6 | 0.935 | 654 | 10390 | 0.10 | 0.52 |
| WebP z9 | 0.680 | 3211 | 7832 | 0.51 | 2.57 |
| zstd 9 palette | 0.814 | 1869 | 9175 | 0.30 | 1.49 |
| zstd 19 palette | 0.729 | 2717 | 8327 | 0.43 | 2.17 |

This world is a poor guide to larger maps: half its LOD 1 pixels are empty, and only 16 + 4 LOD 2/3 tiles exist.

**Map much bigger than the view (estimate):** `Map.loadMapArea` uses `floor(2000 / 500) = 4` tiles of radius at **every**
LOD, unscaled, so 72–81 tiles per LOD (72 measured). LOD 3 alone spans about 112 k blocks, so the full figure needs a map
that wide. Tile sizes from k=8 (repeated terrain): LOD 1 mean 143 KB; full LOD 2 ~515 KB, full LOD 3 ~623 KB (p75, as
edge tiles pull the means down).

| | LOD 1 | LOD 2 | LOD 3 | total | at 50 Mbit/s | at 10 Mbit/s |
|---|---:|---:|---:|---:|---:|---:|
| PNG, 72 tiles each | 10.3 MB | 37.1 MB | 44.9 MB | **~92 MB** | 14.8 s | 74 s |
| oxipng o2 | | | | ~88 MB | saves 0.7 s | saves 3.6 s |
| WebP z6 | | | | ~73 MB | saves 3.1 s | saves 15 s |
| WebP z9 | | | | ~68 MB | saves 3.9 s | saves 19 s |
| zstd 19 palette | | | | ~75 MB | saves 2.7 s | saves 14 s |

Against 9.8 MB measured on the 4096² world. The k=8 map itself would fetch ~51 MB, having only 16 LOD 3 tiles.

## Recommendation

1. **Build no custom codec.** It loses to WebP z9 on size and needs the `src` hook, a decoder and transcoding. Its one
   advantage is encode speed. Keep it as the fallback if WebP's second path proves unworkable.
2. **Change nothing on the write path now.** z9 is ~47× today's encode cost, which DonutSMP ingest cannot absorb.
3. **If large-map load matters, prototype negotiated WebP in `optimized`:** z6 for LOD 2/3 at about 3× encode cost,
   which gets ~0.74–0.79× where a big map's bytes are. Run z9 on cold LOD 1 tiles in the background only. First
   re-measure on real terrain at scale, in Firefox and Safari, and settle the `fetch` path.
4. **Skip oxipng** unless wanted for a one-off rewrite of Java-rendered files: it brings 8–9% at 5× encode cost.

## Surprises

- WebP z8 → z9 drops LOD 1 from 0.94 to 0.66, but LOD 2/3 hardly move. So z9's brute-force search finds a mode the
  heuristics miss on LOD 1. Which mode was not investigated; if it could be forced directly, most of the 2.4 s might go.
- Every plane split or predictor tried made LOD 1 worse than plain whole-pixel LZ.
- Unscaled radius: LOD 2/3 are ~90% of the estimated big-map fetch. That matters more than any codec here, but the
  default view distance is upstream behaviour (see the load breakdown).
- A harness bug, fixed: without the workspace's `zlib-rs` flate2 backend, `encode_png` looked 1.4% larger. With it the
  output totals 1.000× stored.

## Skipped

- Firefox and Safari; a real webapp render with a swapped tile; the `src` hook; a WASM build; `Accept` header capture.
- Zopfli above 5 iterations.
- Whole-map k=8 runs (sampled instead).
