# 21 — Instructions and microarchitecture: `format: optimized` render, end to end

Hardware counters, per-symbol IPC and `perf annotate` of a forced render into optimized file storage, plus
single-thread benches of the ingest and BMQ3-encode stages. Measured 2026-10-22/23 from master `871640d`. Scope:
the crates this round owned (`bm-format` encoder side and PRBM writer, `bm-compress`, `bm-world`, `bm-nbt`,
`bm-math`). Findings in other crates are listed for their owners and were not changed.

## Setup

- testbox (Xeon E-2136, Coffee Lake, 6C/12T, `perf_event_paranoid=1`, NMI watchdog on, so 3 general counters).
  Binaries: `--profile profiling` (release + debug info), generic x86-64 as shipped.
- Fixtures, both `-r -f` into empty optimized storage, 12 render threads:
  - **structures**: `~/bmrs-prof` world, 1153 tiles.
  - **sub**: the 4096² real world of docs/14 cut to its central 2048² by a render mask, 4225 tiles.
- The box ran at load 40–120 from other sessions for most of the work. Instruction counts repeat to ±0.02% and
  are the primary metric; cycles and IPC under that load carry HT-sibling contention. Timings come from a
  `flock ~/bench.lock` run (3 interleaved pairs, medians).
- Harnesses on the box (`~/uarch-bench`, not in the repo): `run.sh` (perf stat / grouped `{cycles,instructions}`
  record / dwarf), `ab.sh` (interleaved A/B with perf stat; checks the hires blobs by decoded bundle digest and
  every other file by bytes). In the repo: `crates/bm-world/examples/chunk_bench.rs` (new) and
  `compact_bench --encode-reps-only` (new flag). Both isolate one phase: `perf stat` at `--reps 4` minus at
  `--reps 1`, divided by 3.

## Whole-render counters (master)

| | structures | sub |
|---|---:|---:|
| instructions | 97.4 G | 337.0 G |
| cycles | 67.3 G (locked) | 231.1 G (locked) |
| IPC | 1.45 | 1.46 |
| branch misses | 0.93% of branches, 1.5 /kinstr | 0.84%, 1.3 /kinstr |
| L1d load misses | | 9.9 /kinstr |
| LLC load misses | | 0.37 /kinstr |
| dTLB / iTLB load misses | | 0.036 / 0.019 /kinstr |

The render is neither branch- nor DRAM-bound overall. Branch misses cost ~3% of cycles at ~15 cycles each. LLC
misses are rare, and TLB misses don't register. IPC 1.45 on a 4-wide core is typical of pointer-heavy code with
L1/L2 latency chains. The low-IPC spots below are each local to one loop.

## Per stage

Self cycles and instructions per symbol from grouped sampling, bucketed by name. BMQ3 `Shapes::build` and `ao`
are inlined into `encode::quads` and counted there.

| stage | structures cycles % | IPC | sub cycles % | IPC | sub after (this branch) cycles % | IPC |
|---|---:|---:|---:|---:|---:|---:|
| block pass + meshing (bm-render) | 36.3 | 1.57 | 41.5 | 1.54 | 42.1 | 1.56 |
| BMQ3 encode (`encode::quads`) | 18.9 | 1.57 | 21.9 | 1.56 | 22.3 | 1.50 |
| volume fill (`column_into`, `Volume::fill`) | 7.5 | 1.64 | 8.0 | 1.64 | 7.7 | 1.74 |
| zstd (BMQ3 frame, level 9) | 5.0 | 1.61 | 6.1 | 1.55 | 6.3 | 1.51 |
| PRBM write | 4.9 | 1.31 | 5.6 | 1.32 | 6.0 | 1.44 |
| lowres + PNG (sub: incl. `Mask::test` 2.6%) | 5.8 | 1.83 | 4.7 | 1.69 | 5.1 | 1.63 |
| NBT + chunk parse | 4.7 | 1.44 | 2.0 | 1.47 | 1.3 | 1.34 |
| chunk inflate | 4.0 | 1.29 | 2.2 | 1.28 | 1.2 | 1.25 |
| libc memcpy/memset/alloc | 4.6 | 0.15 | 3.6 | 0.07 | 3.6 | 0.06 |

From the dwarf call graph of structures (inclusive): BMQ3 encode 18.2%, of which `Shapes::build` 10.2%,
`ao_stream` 3.5%, the exception checks ~2%, `is_quad_shaped` 0.6%. On top of that, zstd 5.6%, lowres PNG encode
5.0%, chunk parse 4.8%, chunk inflate 4.8%, `write_prbm` 3.9%.

## Landed

Locked, 3 interleaved pairs, medians. Output was identical in every run: decoded hires bundle digest, all
other files byte for byte.

| | user CPU | instructions | cycles | wall |
|---|---|---|---|---|
| structures master → branch | 16.28 → 15.43 s (**−5.2%**) | 97.43 → 93.36 G (−4.2%) | 67.30 → 63.73 G (−5.3%) | 2.31 → 2.25 s |
| sub master → branch | 59.63 → 57.90 s (**−2.9%**) | 336.95 → 329.03 G (−2.4%) | 231.09 → 224.62 G (−2.8%) | 6.65 → 6.30 s |

The gain scales with the share of chunk ingest. Structures loads more chunks per tile than the terrain world.
On a 300k² map the ingest share is the terrain one, so expect about −3% of render CPU.

| commit | change | measured (single thread, phase isolated) |
|---|---|---|
| `5110f46` | zlib chunk payloads inflate via pooled libdeflate. Trailing sector padding is ignored, the output guess grows from the reused buffer's last length, and too-small buffers retry at 2×. | inflate pass: instr 4277 → 1580 M (−63%), cycles 3308 → 1445 M (−56%) |
| `f7488f9` | `bm_nbt::Document` records every compound and list length during its one validating walk, and views look them up (binary search) instead of re-walking. `PaletteCache` (per `load_area` split) resolves palette entries by their NBT bytes. | parse pass: instr 4702 → 3151 M (Document) → 2250 M (cache), −52%; cycles −26%, then −27…37% |
| `25ce0b5` | BMQ3 exception rows are compared in place (shifted-self equality for colour and light; normal bytes against the predicted triangle normals). `normal_bytes` is written without `[f32; 3]::map`, and `write_prbm` uses it. | encode pass: instr 35351 → 34321 M (−2.9%); blobs identical |

- **Why the NBT re-walk mattered:** a `Compound`/`List` view only knows where its payload starts. To step past
  one, the parent iterator ran `payload_len` over the whole subtree, once per level the reader descended. A
  palette `Properties` string sat under 7 such walks, and `payload_len` was ~50% of a chunk parse. The index is
  one `(u32 offset, u32 len)` per container. `read_root` without an index stays for
  one-off readers.
- **Palette cache scope:** a rayon `map_init` split only lives for a few chunks. In the render, `intern` and
  `default_state` dropped to 0.1% of cycles. The whole-render gain of the cache alone is ~0.3%, inside run noise.
- **Store-forward stalls** (annotate, before): each exception check wrote its predicted row bytewise (3- or
  6-byte stores) and `pred != row` loaded it back 16 bytes wide. The skid sample sat on the load's next
  instruction, 1–2% of `encode::quads` per stream. The normals check is now an out-of-line closure at about the
  cost `predict_normals` had (compute-bound, flat profile), so cycles moved less than instructions.

## Findings outside these crates (not changed)

- **`Shapes::build` (bm-format `compact/shapes.rs`, under change on another branch): 10.2% of render CPU,
  the largest single BMQ3 cost.** Inclusive:
  - `Arena::find` FxHashMap `[i32; 4]` lookup 1.4%, plus the key's 4 float→int casts 1.1%
  - the `on_grid` test (`.all` over `(l * 16.0) as i32` casts) 1.2%
  - `cell_of` (f64 cross product and floor) 0.9%
  - the `local` f64 round-trip map 0.8%
  - `assign`'s template FxHashMap 0.5%, `uv_id` 0.4%

  Candidates: one key computation shared by both forms, an integer `on_grid` test on the f32 bits (exponent ≥
  −4 and low mantissa bits zero), and skipping the hashed form for axes the plain shape already serves.
  Since done: see [Shapes::build](#shapesbuild) (−36% of its instructions).
- **Volume fill is load-latency bound** (bm-render `view.rs` + `Chunk::column_into`). It reads x → z → y columns
  out of y → z → x sections, so each value is a 256-stride gather. In the annotate, samples sit on the long load
  (14%) and the dependent palette load (13%). Register-resident fields didn't help (reverted, below). A real fix
  is a section-at-a-time fill: decode 16×16×16 in storage order and scatter into the volume. That needs a
  `bm-world` API plus a `bm-render` change.
- **Material sort gather** (`render_tile`, bm-render): IPC 0.6. 29% of its samples sit on one store after
  `src[order[i]]` loads 36-byte records in material order. That is K interleaved forward streams, probably more than
  the L2 streamer tracks. Writing the PRBM straight from the unsorted model in group order (docs/12 round 6) would
  remove the pass.
- **`TileModel` growth:** `realloc` copies are 1.4% of structures CPU (`push_face` extends position/uv/colour Vecs
  from small capacities). A per-thread model with retained capacity removes them. Owners: render / alloc.
- **Low IPC:** `Ctx::tint` 0.64, `Liquid::corner_height` 0.85, `Block::neighbor_light` 0.48. All three are
  dependent loads through state/biome tables.
- **Lowres PNG**: zlib-rs level 9 (`deflate_slow`) is 5% of structures CPU on the 4 persist threads. Bytes are
  pinned by the drop-in rule, so only a byte-identical deflater could cut it.
- **`Mask::test`** (bm-map) costs 2.6% when a render mask is set (the sub fixture).

## x86-64-v3 (information only, not shipped)

Same sources with `-C target-cpu=x86-64-v3`, unlocked:

| | instructions | cycles |
|---|---|---|
| BMQ3 encode pass | −4.8% | ±0 (two pairs: −1.3%, +0.1%) |
| structures render | −4.2% | within noise (+1…3%) |
| sub render | −4.7% | −3.4% (one run) |

docs/12 measured −5.3% CPU for v3 before the libm-floor fixes. What's left is ~3% at most and mostly in
bm-render. Runtime dispatch for the encoder alone isn't worth it: fewer instructions, same cycles.

## Tried and dropped

- **`Padded::strided` with bits/mask/data copied to locals and a running bit shift** (bm-world): samples moved
  from the reloads onto the loads themselves, and `column_into` cycles stayed put (4.86 → 4.97 G). Reverted
  (`b7bcb7e`).
- **NBT lists of fixed-size elements sized as `5 + n × size`:** +0.9% parse instructions. Lists of primitives are
  rare in chunks, and the extra branch costs on every list.
- **`is_quad_shaped` with const-size compares** (`[[u8; N]]` chunks instead of runtime-size slices): +1.1%
  encode instructions.
- **libdeflate output sized from the buffer's capacity:** the zero-fill of a 1 MB reused buffer would cost about
  as much as the inflate. The guess grows from the last length instead.

## Reproduce

```
cargo run --profile profiling -p bm-world --example chunk_bench -- <region dir> --regions 16 --reps 4 --phase parse
cargo run --profile profiling -p bm-format --example compact_bench -- <tiles/0 dir> --step 16 --reps 4 --encode-reps-only
perf stat -e instructions:u,cycles:u,branches:u,branch-misses:u,cache-references:u,cache-misses:u bluemap -c config -v 26.3 -r -f
perf record -e '{cycles:u,instructions:u}' -F 999 …; perf report --group --sort sym --no-children --stdio
```
Each phase cost is the `--reps 4` count minus the `--reps 1` count, divided by 3. Avoid `perf report
--percent-limit 0` on these recordings: it ran for hours, so use the top-60 listing.

## Shapes::build

Second round, 2026-10-23, on master + finest-first shapes (docs/19) + this round's branch (`5f6735f`). Scope:
the BMQ3 encoder's `compact/shapes/` only. Output unchanged throughout: blob hashes `28428af5a839cf27`
(`compact_bench work/exp/real`), `9ad87007f0da7a05` (testbox, every 16th tile of the 4096² render) and
`1cd433a4a56c64a9` (golden fixtures, `--step 2`); `oracle_optimized` passes (14.1× smaller, as before); the real
render's webroots match after conversion to compat (16,641 tiles; the optimized bundles differ only in record
order, which follows the render threads).

Encode instructions per quad (testbox, every 16th tile, 10.18 M quads, `shapes_instr.sh`):

| step | encode instr/quad |
|---|---:|
| base | 3662 |
| key ids recorded per quad by the first pass; u128 keys; f32 local values; bit-test `on_grid` | 3358 |
| SSE2 key and `on_grid` (4 lanes); `build` out of line | 3133 |
| SSE2 shape-fits test | 3053 |
| key ids and picks in one per-quad record (`fbe68fe`) | **3038 (−17.0%)** |

`Shapes::build` itself (`perf record -e instructions:u` share): 1661 → 1068 instr/quad (**−36%**).

| | base | branch |
|---|---:|---:|
| encode cycles/quad, locked (`EVENTS=instructions,cycles LOCK=1`), 3 pairs, median | 1542 | 1279 (**−17%**) |
| encode ns/quad (`compact_bench --encode-reps-only`, locked, 5 pairs, median) | 433 | 292 |
| real render `-r -f` optimized, user CPU s (locked, 3 pairs, median) | 292.1 | 276.0 (**−5.5%**) |
| real render, instructions:u | 1589.4 G | 1489.9 G (**−6.3%**) |

The render saves ~610 instructions per quad, the encode bench's figure. On a 300k² map (~400 render CPU-hours)
that is ~20 CPU-hours. The box ran at load 15–40 from other sessions; ns/quad pairs ranged 351–526 (base) and
284–368 (branch), so treat the timings as direction and the counters as the size.

- **Key ids:** the first pass hashes each (axis, form) key once, records its id in the quad's record and makes
  key id k the first candidate k of the arena. The second pass walks candidates from that id: no hashing and no
  key recomputation. The arena's internal numbering changed (planted in id order, not hash-map order); tables
  are numbered by first use in `assign`, so the output is the same.
- **u128 keys:** rustc-hash 2 hashes `[i32; 4]` and `[u32; 8]` through its byte-slice path; packed integers
  take a multiply per 64 bits (`lanes::pack`). This applies to the key, uv and template maps.
- **f32 local values:** `(f64(v) - f64(b)) as f32` equals `v - b` in f32. f64 has more than 2 × 24 + 2
  significand bits, so rounding the f64 difference to f32 is the f32 difference (no double rounding). The same
  holds for the hashed form's `- d`.
- **SSE2 lanes** (`shapes/lanes.rs`, x86-64 only, scalar twins elsewhere and as the test oracle): the key is
  `cvttps2dq` of `l × 4096`. A lane of `i32::MIN` (NaN, out of range, or exactly −2^31) falls back to the
  saturating scalar cast, so keys keep Rust's `as` semantics. `on_grid` compares truncate-and-back in 4 lanes
  and also accepts |l × 16| = 2^31, the one value the saturating cast maps onto itself and truncation does not.
  `fits` adds and compares 4 vertices at once.
- **`#[inline(never)]` on `build`:** inlined into `encode::quads` it ran +45 instr/quad.
- **Tried and dropped:** last-value caches for uv rows and templates (net ~+85 instr/quad, cause not
  examined); `offer` with its hash path split into a never-inlined function and the hit path
  `inline(always)` (+65); `offer` without its one-entry last-key cache (+13, cache kept).
- **What is left** is flat: `cell_of`'s f64 cross product and floor (~2% of the bench's instruction samples), the uv and
  template hash lookups per quad, and Vec pushes of the per-quad records.

Reproduce (testbox; `~/bmrs-shapes-base` is a checkout of the base):
```
bash docs/perf-exp/shapes_instr.sh <checkout>…                     # encode instr/quad + blob hash
EVENTS=instructions,cycles LOCK=1 bash docs/perf-exp/shapes_instr.sh <base> <new>
bash docs/perf-exp/shapes_bench.sh <base> <new> 5                 # locked ns/quad pairs
bash docs/perf-exp/shapes_render_ab.sh <base> <new> 3             # locked render pairs + output check
```
