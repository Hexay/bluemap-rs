# 07 — Reusing bluemap_reverse (`C:/Users/hexay/bluemap_reverse`)

Assessed 2026-10-06 at commit `3e6b791` (2026-10-02, 55 commits). Paths below are relative to that repo
(`crates/bmr-x/...` → `C:/Users/hexay/bluemap_reverse/crates/bmr-x/...`). Same author, `MIT OR Apache-2.0`.

## Summary verdict

| Asset | LOC (incl. tests) | Tests | Verdict for bluemap-rs |
|---|--:|--:|---|
| `bmr-compress` | 305 | 10 | **REUSE WITH CHANGES** — lz4-java block encode+decode is done and tested; add gzip/zlib/zstd *encoders*, buffer reuse |
| `bmr-prbm` | 1367 | 42 | **REUSE WITH CHANGES** as a **test oracle** (parser + face-level render diff); not a hot-path writer |
| `bmr-world` | 1979 | 26 | **REFERENCE ONLY** for the reader (owned-String serde, per-palette allocs); **reuse writer side in tests** to synthesize worlds |
| `bmr-fetch` | 683 | 4 | **REUSE WITH CHANGES** — `grid.rs` tile paths as-is; `lowres.rs` decoder as oracle; mirror client as webserver conformance test |
| `bmr-seed::java_random` | 75 | 2 | **REUSE AS-IS** (copy) — `java.util.Random` for `Random(2345)` swamp noise |
| `bmr-invert` | 2788 | 57 | **REFERENCE ONLY** — measured facts about BlueMap output (AO formula, tints, UV rotation) |
| `bmr-cli` | 2100 | 1 | **REFERENCE ONLY** — `diff_render.rs`, `check_heights.rs` are the harness drivers to port |
| `bmr-pack`, `bmr-fill`, `bmr-score`, rest of `bmr-seed`, `bmr-cubiomes` | 5,012 | 48 | **NOT RELEVANT** (inversion/reconstruction only) |
| `tools/*.py` (stdlib Python) | 2672 | — | **REUSE WITH CHANGES** — the Java-BlueMap golden harness, server driver, benchmark rig |
| `fixtures/*/fixture.json` | 16 | — | **REUSE AS-IS** (copy) — world specs + `commands.txt` |
| `work/bluemap/<fx>/web/maps/` | — | — | **Golden outputs already on disk** (BlueMap 5.27, MC 26.3); regenerate for 5.28 |

## 1. What bluemap_reverse does end to end

Goal (`README.md:5`, `docs/architecture.md:3`): from a BlueMap website URL, rebuild a playable Java world.
`bmr pull <url> -o world.zip`: **mirror** settings/textures/hires/lowres → **decode** PRBM → **invert** faces to
block states by matching against signatures *learned from BlueMap's own render of the debug world*
(`architecture.md:32-37`) → **fill** hidden volume (evidence, priors, or seeded regen) + biomes from tints →
**write** Anvil regions per dimension, zip or Sponge `.schem` (`README.md:121-126`). Seed cracking from detected
structures (`bmr-seed` + vendored cubiomes). Pinned: MC 26.3 (+ packs for 1.21.4/.8/.11), BlueMap 5.27
(`60e733f`), Java 25, Rust 1.88, edition 2024 (`architecture.md:9-16`).

Accuracy is measured, not claimed: fixture worlds are built by a real server, rendered + served by real BlueMap
CLI, mirrored over HTTP, reversed, then scored block-by-block (`docs/results/*.json`, `history.jsonl`) and
**re-rendered and diffed face-by-face** (`tools/roundtrip.py`, `docs/results/*-roundtrip.json`).

`docs/performance.md`: `tools/bench.py` (median wall/CPU/peak working set + per-stage timings →
`docs/results/bench.jsonl`), `tools/profile.py` (samply/ETW), `tools/check_equiv.py` (bit-identical output
guard for optimisations). Decisions: intern block states as `u32` (`performance.md:21` — 45 % CPU was string
clone/hash), FxHash, uniform sections store no index array (`:25`), region-windowed pipeline with 32-block halo
so memory doesn't scale with map (`:26`). vanilla-512 bench: 39 s/1.5 GB → 7 s/375 MB (`bench.jsonl` tail).

Crates (`architecture.md:71-96`, leaves first): `bmr-compress` storage + chunk decompression · `bmr-prbm` PRBM
parse, OBJ export, render diff · `bmr-world` Anvil read/write, registry, `.schem` · `bmr-fetch` site mirror ·
`bmr-invert` signature matching/evidence · `bmr-fill` hidden-volume fill, game rules, biomes · `bmr-pack`
versioned signature packs · `bmr-score` world diff · `bmr-seed` structure-seed crack · `bmr-cubiomes` C FFI ·
`bmr-cli` the `bmr` binary. 190 `#[test]`s total; a few `#[ignore]` need `work/data`.

Fixtures (`fixtures/*/fixture.json`: server.properties overrides + `area` + optional `commands.txt` +
optional `"bluemap"` map-conf overrides): `superflat`, `superflat-bare`, `superflat-lz4`
(`region-file-compression=lz4`), `debug` (every block state), `context` (states among neighbours: stairs,
fences, redstone, doors, waterlogging — generator ports vanilla neighbour rules, `tools/context_rules.py`),
`vanilla` (256², seed `bluemap_reverse`), `vanilla-512`, `vanilla-edited`, `nether`, `end`, `dimensions`
(one world, 3 maps), `biomes` (one patch per biome), `structures*`, `template-void`, `seed/*.json`.

## 2. Per crate

### bmr-compress — REUSE WITH CHANGES
- API: `Format::{Gzip,Zlib,Zstd,Lz4Block}`, `Format::sniff` by magic (`src/lib.rs:23-31`),
  `Format::decompress(data, limit)` (`:33`), `decompress_any` (`:47`); every decoder output-capped.
- `lz4_block.rs`: lz4-java `LZ4BlockOutputStream` framing — 21-byte header, 64 KiB blocks, RAW/LZ4 method,
  XXHash32 seed `0x9747b28c` masked to 28 bits (`:9-21`), **`compress`** (`:32-42`) and hardened
  **`decompress`** (`:45-85`, bounds + checksum). Tested incl. region-embedded lz4 (`bmr-world/src/region/tests.rs:107-125`).
  This is exactly the "custom parser over lz4_flex" item of Phase 1 and BlueMap's `lz4` storage compression.
- Deps: flate2, lz4_flex (block, safe/checked), ruzstd (pure-Rust **decode only**), twox-hash.
- Gaps for bluemap-rs: no gzip/zlib/zstd encoders (storage write path); returns fresh `Vec` (want
  `decompress_into(&mut Vec)` for per-thread buffer reuse); `anyhow` in a library API (switch to a typed error);
  consider `zlib-rs` backend for flate2 and the `zstd` crate (encoder + faster decoder) behind a feature.
  lz4_flex output is decodable by lz4-java but not byte-identical to its JNI/Java compressor — fine (decode-compat
  is the contract), note it in tests.

### bmr-prbm — REUSE WITH CHANGES (test oracle)
- `parse(buf) -> Tile` (`src/parse.rs:22-52`): validates version 1, non-indexed LE, looks up attributes **by name
  and type-checks** encoding/cardinality (`:54-67`, `:88-111`), groups contiguous and covering all vertices, no
  trailing bytes (`:69-86`). Strict = good oracle: a malformed writer output fails loudly.
- `Tile` SoA vectors + `Group{material,start,count}`; `Tile::faces()` → `Face` per triangle (`src/tile.rs:4-68`).
- `textures.rs`: serde `textures.json` (`resourcePath,color,halfTransparent,texture,animation`), base64 PNG,
  first-frame crop (`:9-74`).
- **`diff/`** (`src/diff/mod.rs:20-229`): `RenderDiff::add_tile(original, names, other, names, origin, unrendered)`
  pairs faces by geometry quantized to 1/4096 block with sorted vertex order, compares texture **by name**
  (material ids may differ), uv, tint, AO, blocklight, sunlight; reports missing/extra by texture, per-aspect mean
  |Δ|, worst cells (`report.rs`). This *is* the "geometry-level compare, byte-exact impractical" golden check
  from 00-overview Phase 0 — already proven on real BlueMap output (99.99 % identical original-vs-relit).
- `test_prbm.rs:3-67`: test-only PRBM **byte builder** in `PRBMWriter.java` layout (attr flag bytes `0x21 0x63
  0x67 0x11 0x47 0x03 0x03`, padding from file start, `-1` terminator) — seed for our writer's tests.
- No writer. Parser allocates (HashMap of attrs, owned Vecs): fine for tests, not for a server hot path.
- Deps: base64, png, serde_json. Port: keep `parse`/`Tile`/`textures`/`diff`, drop `obj.rs`; write
  `PRBMWriter` fresh in bluemap-rs (single `Vec<u8>`, bucketed by material) and round-trip it through `parse`.

### bmr-world — REFERENCE ONLY (reader) / test utility (writer)
- Region: `read_region_where(path, keep)` reads the **whole file** (`std::fs::read`) then rayon-decompresses
  kept slots (`src/region.rs:21-39`); compression 1/2/3/4, `.mcc` external chunks **rejected** (`:80`).
  `write_region` zlib-only (`:42-71`).
- Chunk decode: `fastnbt::from_bytes` into serde structs with owned `String`s (`src/nbt.rs:12-57`, `:82-102`);
  each palette entry → `BlockState{name: String, properties: Vec<(String,String)>}` (`src/chunk.rs:5-9`). That is
  the very per-section allocation pattern bluemap-rs is replacing (simdnbt borrow mode + global ids).
  Reads only `block_states` + `biomes`: **no light, heightmaps, block entities, entities**; 1.18+ only.
- Worth lifting as knowledge/small code: 26.3 palette shorthand semantics — bare name = default state, mixed
  lists wrap as `{"": name}`, `{id, properties}` vs legacy `{Name, Properties}` (`nbt.rs:40-80`); paletted
  `unpack` (no long-spanning, min bits, single-entry → no data; `nbt.rs:104-119`); dimension folder mapping
  26.1+ `dimensions/<ns>/<name>` vs `DIM-1/DIM1` (`src/world.rs:24-37`); `BlockRegistry` from the vanilla
  `blocks.json` data report (`src/registry.rs:37`) — bluemap-rs needs defaults for the 26.3 shorthand too.
- `StateTable`/`StateId(u32)` (`src/states.rs:8-49`): per-run interner keyed by owned `BlockState`. Same idea as
  bluemap-rs's global ids but not the structure we want (no dense property/face tables, string-keyed).
- Writer (`nbt_write.rs`, `world_write.rs`, `sparse.rs` `ChunkBuilder` on `StateId`s, server-accepted for 26.3 and
  legacy palettes — `tools/check_writer.py`): useful **in tests** to synthesize tiny worlds for mesher unit tests
  without spinning up a server. Deps: fastnbt, flate2, rayon, rustc-hash, zip.

### bmr-fetch — REUSE WITH CHANGES
- `grid.rs`: `Grid{size,offset}`, `tile_of`, `tile_min`, `tiles_in`, `tile_path` digit split `x-1/2/z3/4/5`,
  `tile_file(lod)` (`src/grid.rs:5-56`, tested) — reuse as-is (add `.gz` suffix handling on the storage side).
- `lowres.rs`: `LowresImage::decode` + `color`/`block_height` (G:B BE i16) (`src/lowres.rs:7-52`) — oracle only;
  bluemap-rs needs the encoder plus blocklight (R) and the +1 seam row.
- `settings.rs`: partial serde of root/map `settings.json` (`src/settings.rs:7-71`) — read-only subset; we must
  emit the full upstream shape.
- `http.rs`/`mirror.rs`/`discover.rs`: polite crawler (`Accept-Encoding: gzip`, 204/404 = empty, decompress by
  magic, `src/http.rs:63-74`), tile enumeration from lowres extent + probing, manifest. Reuse as a **webserver
  conformance client**: mirror Java BlueMap's server and ours, then byte-compare (`tools/verify_mirror.py`).
  Deps: ureq (native-tls), png, rayon.

### bmr-seed — copy `java_random.rs` only
`JavaRandom::{new,next,next_int,next_long}`, `string_hash`, verified against OpenJDK values
(`src/java_random.rs:10-68`). Needed for `Random(2345)` swamp-foliage noise (00-overview Phase 2). Rest (structure
detection, lower-48/upper-16 seed solve) is not relevant.

### bmr-invert — REFERENCE ONLY
Signature library/matcher/evidence are inversion logic. Useful *measured facts* about BlueMap 5.27 output:
vertex AO = 255 − 64 × full blocks among the two side cells + diagonal in the facing layer, no exceptions on
vanilla-edited (`src/ao.rs:1-5`); textures rotated by position hash for stone/grass/kelp (`src/lookalike.rs:2`,
`src/face.rs:2`); per-biome grass/foliage/water tints learned from the `biomes` fixture (`src/tints.rs`). Treat as
cross-checks for our mesher, not code to import.

### bmr-cli — REFERENCE ONLY
`diff-render` (`src/diff_render.rs:56-86`: two mirrors → `RenderDiff`, `--inset`, `--json`), `check-heights`
(`src/check_heights.rs:1-2`: hires top faces vs lowres lod-1 heights, 100 % on superflat), `obj` export, region
windowing with halo (`src/window.rs`). Port the first two as bluemap-rs golden-test commands.

### NOT RELEVANT
`bmr-pack` (postcard+xz signature packs, texture-fingerprint index), `bmr-fill` (gap fill, stairs/redstone/note
rules, regen merge), `bmr-score` (voxel accuracy between two worlds), `bmr-cubiomes` (C FFI for seed upper bits).

## 3. Especially valuable for bluemap-rs

**Java BlueMap golden harness (tools/, stdlib Python, Windows + Linux):**
- `setup.py`: per toolchain, downloads Adoptium JDK (`paths.py:31`), Mojang server jar via
  `version_manifest_v2` with **SHA-1 check** (`setup.py:32-59`, `:101-103`), BlueMap CLI jar from GitHub
  releases (`paths.py:63`), generates vanilla data reports (`blocks.json`) (`setup.py:106-116`). Detects the
  Java each needs: server from Mojang metadata, BlueMap from its class-file major (`setup.py:62-76`).
- `make_world.py` + `console.py`: headless server over stdin, force-load area in ≤256-chunk batches, apply
  `commands.txt`, save. Gotchas baked in: `pause-when-empty-seconds=0`, `max-tick-time=-1`
  (`make_world.py:15-27`), escaped `:` in properties (`:49`), shared `-DbundlerRepoDir` (`performance.md:37`).
- `render_serve.py`: creates BlueMap config via first run, sets `accept-download`, `metrics=false`,
  **`render-thread-count`=all cores** (default 1: 340 s → 25 s), webserver ip/port, one map conf per dimension
  using BlueMap's own default templates + fixture overrides (`render_serve.py:42-77`); runs `-r [-f] -w`;
  `--world` renders any other world with the same map config; `--relight` resaves first (BlueMap darkens/skips
  unlit chunks, `:9`).
- `mirror_fixture.py` / `verify_mirror.py`: mirror over HTTP; byte-compare mirror vs on-disk webroot after gunzip.
- `roundtrip.py`: relight → render → `bmr diff-render` with `INSET=16` (sky light leaks 15 blocks sideways at
  a world's edge, `roundtrip.py:6-9`, `architecture.md:221-224`) — bluemap-rs needs the same inset when diffing
  partial worlds.
- `check_version.py`: per-MC-version end-to-end; logs `work/check_{26.3,1.21.11,1.21.8}.log` all end
  "server … loads the output: OK" (score 99.92 %). Packs also built for 1.21.4 (`work/build_pack_*.log`).
  Matrix today = 1.21.4–26.3 only; supports any 1.18+ by flag. For bluemap-rs's 1.13.2+ range, extend
  `setup.py` (older servers need Java 8/17 — already derived from Mojang metadata) and `make_world.py`
  (pre-1.21.2 properties).
- `bench.py` (CPU time + peak working set + stage JSON → jsonl), `profile.py` (samply), `check_equiv.py`
  (bit-identical regression guard), `testbox.py` (offload RAM-heavy runs over ssh to a 31 GB Linux box).

**Golden outputs already on disk** (BlueMap 5.27 / MC 26.3, `storage: file`, `compression: gzip`):
`work/bluemap/<fixture>/web/maps/<id>/{settings.json, textures.json.gz, tiles/0..3, rstate/, live/}` for 14 maps
(superflat 65 files … structures 1,246; `debug` = every block state). `rstate/` (tiles + `regions/`) is present —
first real samples to test our render-state NBT reader against. Source worlds in `work/worlds/<fixture>/world`,
per-version under `work/v/<mc>/`. Re-render with 5.28 before treating them as the 84ee993 contract.

**Mojang client jar:** the tools never fetch it; BlueMap does (accept-download) into
`work/bluemap/<fx>/data/minecraft-client-26.3.jar` next to `resourceExtensions.zip` — handy fixtures for Phase 2
resource loading. Rust client-jar fetch must be written fresh; `setup.py:54-59` shows the manifest → version
JSON → `downloads.<side>.{url,sha1}` path (swap `server` for `client`).

**Format oracles:** `bmr_prbm::parse` (strict), `RenderDiff`, `LowresImage`, `bmr_compress` decoders,
`tile_path`, `check-heights` (hires/lowres consistency) — all validated against real BlueMap output.

**Docs:** `docs/research/01-bluemap-web-format.md` (PRBM table §2, lowres §5, serving §1) and
`03-rust-and-tooling.md` (chunk NBT shape, BlueMap CLI usage, test-world generation) complement our 03/04.

## 4. Overlap vs divergence

Genuinely shared (same bytes, same semantics): lz4-java block framing; gzip/zlib/zstd sniffing; Anvil region
header + compression byte; 1.18+ paletted-container unpacking; 26.3 palette shorthand + `blocks.json` defaults;
26.1+ dimension folders; tile grid/digit-split paths; PRBM layout; lowres PNG layout; `textures.json` schema;
`settings.json`; webserver behaviour (204, gzip passthrough); java.util.Random; the Java-BlueMap test harness.

Diverges because reverse optimizes for inversion, not forward throughput:
- **Direction**: reverse *reads* PRBM/lowres/textures and *writes* Anvil; bluemap-rs *reads* Anvil and *writes*
  PRBM/lowres/textures/rstate. Only compression is bidirectional in both.
- **World reader**: whole-file read, owned-String serde, per-run interner, blocks+biomes only. bluemap-rs needs
  cached handles + positioned reads, borrow-mode NBT, global dense ids, light/heightmaps/block entities,
  `.mcc`, and 1.13–1.17 decoders.
- **Version range**: reverse 1.18+ (practically 1.21.4+); bluemap-rs 1.13.2–26.x.
- **Data model**: reverse works on face sets, evidence and voxel maps windowed per region; bluemap-rs on flat
  per-tile block arrays and baked per-state geometry.
- **Not covered at all**: render state (rstate), SQL storage, HOCON config, resource-pack/model baking, mesher.
- **Errors**: `anyhow` everywhere (apps) vs typed errors wanted in bluemap-rs library crates.

## 5. Recommendation: how to share

1. **Copy, don't depend — now.** The reusable code is ~700 LOC (`lz4_block.rs` + tests, `Format::sniff`,
   `java_random.rs`, `grid.rs`, `prbm` parse/reader/tile/textures/diff, `unpack`, `test_prbm.rs`, `lowres.rs`).
   A git dependency on bluemap_reverse would drag `anyhow` APIs and couple two moving projects for little code.
   Same author and `MIT OR Apache-2.0`, so copying needs no notice beyond keeping headers; if bluemap-rs is
   MIT-only (BlueMap is MIT), dual→MIT is the author's call. cubiomes (MIT) is not needed.
   Placement: `lz4_block` + codecs → `bm-storage` (or `bm-compress`); `java_random` → `bm-resources`;
   `grid` → shared `bm-format`; PRBM parser + `RenderDiff` + lowres decoder → a dev-only `bm-golden` crate.
2. **Later, invert the dependency.** Once bluemap-rs has a stable `bm-format` crate (PRBM read+write, lowres
   read+write, tile paths, textures.json, settings.json, storage codecs incl. lz4-java), publish it (crates.io or
   git) and let bluemap_reverse depend on it — the forward renderer is the format's source of truth. Don't
   build a third "common" workspace.
3. **Harness:** copy `tools/{paths,setup,console,make_world,render_serve,mirror_fixture,verify_mirror,bench}.py`
   and `fixtures/` into `bluemap-rs/tools` + `bluemap-rs/fixtures`; add a "render with bluemap-rs" step next to
   `render_serve` and a `golden` command that runs `RenderDiff` + lowres compare + rstate/`textures.json` diff
   per fixture. Point `DOWNLOADS` at a shared cache (env var) to reuse the existing JDKs/server jars/BlueMap jar
   in `bluemap_reverse/work/downloads` instead of re-downloading. Bump the default toolchain to BlueMap 5.28.
4. **Bench:** reuse `bench.py` unchanged against both `java -jar bluemap-cli.jar -r -f` and our binary on
   `vanilla-512` / `structures` worlds — it already records CPU time and peak working set, the two numbers the
   rewrite is selling.
