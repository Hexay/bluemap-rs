# BlueMap → Rust: overview & plan

Reference source: BlueMap @ `84ee993` (2026-10-05), BlueMapAPI v2.8.1, latest release v5.28 (MC 1.13.2 – 26.3, Java 25).
Subsystem deep-dives:

| # | File | Covers |
|---|------|--------|
| 01 | [01-world-reading.md](01-world-reading.md) | Anvil/MCA, NBT, chunk versions, palettes, caches, dimensions |
| 02 | [02-resources.md](02-resources.md) | Client jar, pack layering, blockstates, models, textures, tints, biomes |
| 03 | [03-rendering.md](03-rendering.md) | Tile grid, hires mesher, liquids, AO/light, lowres, PRBM, render state, masks |
| 04 | [04-storage-web.md](04-storage-web.md) | File/SQL storage, compression, HTTP server, **webapp contract**, live/SSE |
| 05 | [05-orchestration-platforms.md](05-orchestration-platforms.md) | Lifecycle, render manager, configs, CLI, platform layers, API, addons |
| 06 | [06-prior-art-and-ecosystem.md](06-prior-art-and-ecosystem.md) | Known perf pain, Rust crates, format drift, JVM interop, license |
| 07 | [07-bluemap-reverse-reuse.md](07-bluemap-reverse-reuse.md) | What to take from `C:/Users/hexay/bluemap_reverse` (golden harness, diff oracle, lz4-java, Java Random) |
| 08 | [08-github-issues.md](08-github-issues.md) | All 628 upstream issues categorised: real pain points, bugs not to copy, behaviour to keep |
| 12 | [12-perf-profile-rs.md](12-perf-profile-rs.md) | Profiled our engine: gzip 22%, Volume::fill 11%, soft floor 6% (last two fixed, −10.8% CPU); disk/web hotspots |

## What BlueMap is

A 3D web map for Minecraft. It reads world save files directly (never the live server's memory),
meshes every 32×32-block column into a 3D tile, derives 2D heightmap/colour tiles for distant
zoom levels, stores both, and serves a Vue + three.js webapp that streams those tiles.
It runs as a Paper/Spigot/Sponge plugin, Fabric/Forge/NeoForge mod, or standalone CLI.

## Product goal: drop-in replacement

Switching must cost the user ~nothing: swap the jar/binary, restart, keep everything. Concretely:

| User has… | Drop-in means… |
|-----------|----------------|
| `config/bluemap/*.conf`, `maps/*.conf`, `storages/*.conf` | Read unchanged — same HOCON keys, defaults, hidden keys, same config/data/webroot folders |
| Already-rendered maps (files or SQL) | Served as-is **and incrementally updated** — read existing `rstate/*.dat` (gzip NBT) + `textures.json` ids; no forced re-render |
| `data/pluginState.json`, `tasks.dat` | Read; resume queued tasks and paused/frozen state |
| nginx/apache/`sql.php` external webserver setups | Keep working — identical paths, suffixes, SQL schema, compression ids |
| Customised `web/` (settings.json, scripts, styles) | Untouched; same webapp version shipped |
| CLI scripts / systemd units | Same flags (`-r -u -w -f -e -c -d …`), exit codes (1 config, 2 missing resources) |
| Marker/integration plugins (BlueMapAPI consumers) | Keep working — our jar ships the real `de.bluecolored.bluemap.api` classes and declares `provides: [BlueMap]` (Paper `plugin.yml`) / `provides: ["bluemap"]` (`fabric.mod.json`) so `depend: BlueMap` resolves |
| `/bluemap …` commands + permissions | Same command tree and permission nodes |
| Java addons (`packs/*.jar` with `bluemap.addon.json`) | **Cannot run** on a Rust core — detect, warn clearly, still load their bundled resource packs |

Consequences:
- **Visual parity over cleanup**: newly rendered tiles sit next to Java-rendered ones, so replicate upstream quirks
  (Java `Random`, TrigMath, `variants[0]` fallback, `dry_foliage` overlay) — no seams.
- **Track upstream releases**: each of our releases names the BlueMap version it is drop-in for; ship that webapp build.
- Native/off-heap memory counts against the container limit, not `-Xmx` — users on tight hosts may need to lower `-Xmx` slightly (document in migration notes).
- Some hosts mount temp dirs `noexec`; extract the native binary under the plugin data folder, not `/tmp`.

## Data flow

```
config (HOCON) ──► BlueMapService
                     │
 client jar (Mojang) ┼─► ResourcePack ──► baked blockstates/models, textures.json, blockColors, biomes
 packs/, mod jars,   │                            │
 world datapacks ────┘                            ▼
 world/region/*.mca ──► World/Chunk (NBT) ──► HiresModelManager ──► ArrayTileModel ──► PRBM (.prbm.gz)
   ▲ mtime/timestamps        (8×8×8 neighbour       │                                     │
   │                          lookups, light,       └─► per-column height+colour ──► LowresTileManager
 MapUpdateService             biome tint)                                              ──► PNG LOD1..3
 (file watcher, 5-min                                                                      │
  fingerprint, 24h full) ──► RenderManager (single task list, N threads, one tile per call)  ▼
                                                                      Storage (file tree | SQL blobs, gzip default)
                                                                                           │
 Platform layer: players, worlds, save-now, commands ──► live/players.json, markers.json, SSE
                                                                                           ▼
                                                        Web server (hand-rolled NIO) ──► webapp (three.js)
```

## Compatibility contracts (must be byte-exact to reuse the webapp)

1. **Hires tile = PRBM** — little-endian, non-indexed, 7 attributes in fixed order
   (position f32, normal i8, color u8, uv f32, ao u8, blocklight i8, sunlight i8), material groups
   terminated by -1. Writer `PRBMWriter.java:62-277`, parser `PRBMLoader.js:112-252`. See 03 §2, 04 §8.
2. **Lowres tile** — 501×1002 PNG: top half RGBA colour, bottom half R=blocklight, G:B=signed 16-bit height.
3. **Paths** — `maps/<id>/tiles/0/x-1/2/z5.prbm.gz` (dir per digit), `tiles/<lod>/….png`,
   `maps/<id>/settings.json`, `textures.json`, `live/players.json|markers.json`, `live/sse`.
   Missing tile → HTTP 204. Pre-compressed bytes passed through with `Content-Encoding`.
4. **textures.json** — array index = material id in PRBM; ids are stable across runs (existing file loaded first).
5. **SQL schema** — 6 tables, `bluemap_` prefix hard-coded in `public/sql.php`; keep identical.
6. **Render state** — `rstate/*.dat` gzipped NBT (tile/chunk/region state); read *and* write it so incremental
   updates continue over a Java-rendered map and users can switch back. Layout in 03 §7, 04 §1.
7. **BlueMapAPI** (Java) — markers, render triggers, webapp scripts/styles, assets. Consumed by third-party plugins.

## Where the performance & memory gains actually are

Ranked by expected payoff (details in 01/02/03):

1. **Global interned block-state ids.** Java allocates a fresh `BlockState` (map + key) per palette entry
   per section, then hashes it into a Caffeine cache per block. Rust: `u32` state id → dense tables of
   properties, baked faces, colour behaviour. Removes most allocation and hashing.
2. **Pre-baked geometry per state.** Java re-resolves variants/multipart every block and transforms vertices
   4–5×. Rust: resolve once per state at load, store transformed quads; per block only translate + cull.
3. **Tile-local flat block arrays.** Java does ~100 neighbour lookups per opaque block, each through a
   hashed chunk cache. Rust: copy the tile (+1 border) into a flat `[u32]` once, neighbours become index math.
4. **Biome tint once per tile** with a separable box blur instead of 75 samples per tinted block.
5. **Chunk I/O.** Java opens/closes the region file per chunk, no buffer reuse, Caffeine soft cache up to
   ~10k chunks (1–2 GB). Rust: cached file handle + positioned reads, per-region working set, simdnbt
   borrow-mode parsing (no tree), only decode sections the tile needs.
6. **Output.** Build PRBM in one `Vec<u8>`, bucket triangles by material as emitted (no sort), compress once
   (zlib-rs / libdeflate). Lowres PNG via fast encoder.
7. **Parallelism.** rayon over tiles inside a region, per-thread scratch arenas, no shared mutable caches.
8. **Memory isolation.** Out of the server JVM heap entirely if run as a separate process.

Prior-art sanity check: MinedMap (Rust, 2D) renders a 3 GB save in <5 min single-threaded under 100 MB.
BlueMap's only published throughput: ~24–44 tiles/s on 32 threads (2020). **We need our own benchmark rig.**

## Key architectural decision: how to run on servers

Rendering never needs the JVM — BlueMap already reads region files from disk and detects changes by file
watching (no chunk events). The Java side only supplies: paths/version, world list, block default-state
dump, player snapshots (~1 Hz), "save now", commands+permissions, and the public API. Options (05 §4, 06 §5):

| Option | Pros | Cons |
|--------|------|------|
| **A. Standalone Rust binary** (CLI + webserver) | Simplest, crash-isolated, own memory | No live players/markers/API |
| **B. Rust process + thin Java shim over IPC** | Crash-isolated, RAM outside server heap, API proxyable | IPC protocol to design, process supervision |
| C. In-process via FFM (Java 25) / JNI | Single artifact | Rust crash kills server; JDK 24+ native-access warnings; shares process RAM |

**Recommendation:** build A first (it *is* the core, and is already drop-in for CLI/Docker users), then B
packaged as a single jar that bundles per-OS Rust binaries and spawns them — the user still installs one jar.
Defer C.

## Things we will have to do (work breakdown)

Phase 0 — foundation
- Cargo workspace: `bm-nbt`?/`bm-world`, `bm-resources`, `bm-render`, `bm-storage`, `bm-web`, `bm-config`, `bm-cli`.
- **Golden-test harness**: run Java BlueMap CLI on fixed test worlds, capture PRBM/PNG/JSON output, diff ours
  (geometry-level compare; byte-exact is impractical due to Java float/TrigMath quirks — see 03 §12).
  Already exists in bluemap_reverse (`tools/` harness + `bmr-prbm` face diff) — port it, see 07.
- Benchmark rig: fixed worlds of several sizes, measure tiles/s, peak RSS, vs Java.

Phase 1 — read the world
- Region reader: compression 1/2/3/4 (lz4-java block framing — custom parser over lz4_flex), `.mcc` externals, 127 = skip/warn.
- Chunk decoders by DataVersion: 1.13 / 1.15 / 1.16 / 1.18+ / 26.3 palette rename (`Name`→`id`, string palettes) / 26.4 per-block biomes.
- Light, `OCEAN_FLOOR` heightmap, block entities (signs, skulls, banners), 1.17+ entities.
- Dimensions: `dimensions/<ns>/<path>` (26.1+, keys may contain `/`), legacy `DIM-1/DIM1`, dimension types from level.dat/datapacks.

Phase 2 — resources
- Mojang version-manifest download + SHA-1 (gated behind `accept-download`).
- Pack layering exactly as 02 §2 (first wins, atlases merge, nested jars/overlays).
- Lenient JSON (comments, trailing commas, unquoted, coercions) — pre-pass or `json5`-style parser.
- Blockstate variants/multipart (incl. BlueMap's `variants[0]` fallback), weighted pick with Java 64-bit hash.
- Model parent merge, texture ref resolution, bundled `resourceExtensions.zip` (≈199 hand-made entity-ish models).
- Textures: average colour, half-transparency, animation; `textures.json` with stable ids.
- Block colours incl. swamp noise seeded by **`java.util.Random(2345)`** — reimplement Java LCG exactly.
- `blockProperties.json` tri-state flags.

Phase 3 — render
- Hires mesher (03 §4–5): culling, AO, light, tint, waterlogging, liquids, random offset, cave removal, masks, edges.
- PRBM writer; lowres column extraction, LOD cascade (factor 5, 3 levels), PNG.
- Render-state tracking via region-header timestamps; tile state machine.

Phase 4 — storage & web
- File storage (exact path scheme) + SQL via sqlx (MySQL/MariaDB/Postgres/SQLite), compression none/gzip/deflate/zstd (lz4 optional).
- axum server: pre-compressed passthrough, 204 for missing, `no-store` live JSON, SSE, real Content-Length.
- Embed the unmodified webapp build (rust-embed), on-disk overrides win.
- Fix upstream webserver DoS (bounded request bodies) for free.

Phase 5 — orchestration
- HOCON config (likely own parser; `hocon` crate unmaintained) with identical keys/defaults (05 §3).
- Render manager + tasks, file watcher (notify crate) with 5s debounce/60s cooldown, 5-min fingerprint, 24h full.
- CLI flags compatible with BlueMapCLI (`-r -u -w -f -e …`).

Phase 6 — server integration (option B)
- Thin Java plugin (Paper + Fabric first): world list, block-state dump, players, save-now, commands, IPC.
- BlueMapAPI shim: markers serialised to JSON every 10s and pushed over IPC; render triggers; assets.
  `setTileFilter` (per-tile Java predicate) is hard — likely unsupported or batched.

## Decisions

Settled by the drop-in goal:
- Upstream bug parity — **replicate** (no seams between Java- and Rust-rendered tiles).
- Webapp — **ship whatever the targeted BlueMap release ships**, follow it if the rewrite (#864) lands.
- Render state / textures.json / SQL schema — **read and write upstream formats**, switching back stays possible.

- Targets — **both CLI/Docker and server plugins**: the core ships as the CLI first; the Paper jar
  (then Fabric/Forge/NeoForge) wraps the same binary. Sequencing only, not a choice.

- Storage modes — **`compat`** (upstream layout, byte-exact) and **`optimized`** (packed bundles + compact quad
  encoding + zstd, transcoded back to exact PRBM on serve; ~7.5× smaller, see 09). Webapp unchanged in both;
  `storage convert` moves between them. Optional later: `optimized-static` with a patched webapp tile loader.
- Storage default — **new installs `optimized`; existing BlueMap storages stay `compat`** until the user converts.
- Server integration — **separate process**: one plugin jar bundles per-OS Rust binaries, extracts to the plugin
  data folder, spawns and supervises the core, talks over local IPC. No in-process FFM/JNI.
- Java addons — **warn only** for the port: detect `packs/*.jar` with `bluemap.addon.json`, log each by name, still
  load bundled resource packs. Expected long-term path: rebuild popular addons natively, after the port is done.

- Web server — **axum/hyper/tokio**, not a port of the hand-rolled NIO server (its parser is the source of #737,
  #828, #750, #527). Wire behaviour identical to upstream (04 §3–4); webroot via tower-http `ServeDir` + rust-embed
  fallback, map data via custom storage-backed handlers.
- MC range — **all chunk formats 1.13.2–26.x** (current worlds still hold never-reloaded old chunks, e.g. #521);
  build 1.18+ first (harness coverage), add 1.13–1.17 before first release.

Still open:
- **Public name** — working name `bluemap-rs`, crates `bm-*`. "BlueMap" is not covered by the MIT license; pick a
  distinct name before release; `provides: BlueMap` keeps plugin deps working; keep MIT notices.

## Risks

- Upstream moves fast (26.x format changes every few releases; webapp rewrite pending) — pin a reference commit and track diffs.
- Exact visual parity relies on Java-specific maths (Random LCG, TrigMath, float casts).
- Ecosystem lock-in: users rely on API-based marker plugins and addons; a core without the shim is a CLI-only product.
