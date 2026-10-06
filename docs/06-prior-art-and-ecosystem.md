# 06 — Prior art, ecosystem, format drift, interop, license

Researched 2026-10-06. Every non-obvious claim has a URL. "Unverified" = secondary source only.

## 1. Current BlueMap baseline

- Latest: **v5.28, 2026-09-25**, supports **MC 1.13.2 – 26.3**; plugin/mod builds need **Java 25**.
  https://github.com/BlueMap-Minecraft/BlueMap/releases/tag/v5.28
  - 5.28 notes: SQL `table-prefix`, `connection-init-sql` (WAL default for SQLite), "render-state caching
    and faster block-state parsing" for map updates.
- Install page: Java 25 for most targets, **CLI says Java 21+** (inconsistent; treat 25 as reality).
  https://bluemap.bluecolored.de/wiki/getting-started/Installation.html
- Platforms (repo `implementations/`): cli, fabric, forge, neoforge, paper, spigot, sponge. Docker image
  (`ghcr.io/bluemap-minecraft/bluemap`). https://github.com/BlueMap-Minecraft/BlueMap/tree/master/implementations
- BlueMapAPI latest **v2.8.1** (2026-09-25). https://github.com/BlueMap-Minecraft/BlueMapAPI
- Repo: MIT, ~2.8k stars, ~430 Java files, active (push 2026-10-05).
- Webapp: Vue 3.5 + three.js r186 + Vite 6 (`common/webapp/package.json`). A **webapp rewrite is in
  progress** (~1 yr, no ETA) — the port's web target may move. https://github.com/BlueMap-Minecraft/BlueMap/issues/864

## 2. Known performance / memory pain points

| Pain point | Evidence |
|---|---|
| "Uses too much RAM" perception — JVM heap growth + soft-ref caches of chunk data/resources; no way to cap BlueMap separately from server heap | https://bluemap.bluecolored.de/community/RAM.html, https://github.com/BlueMap-Minecraft/BlueMap/issues/565 |
| Official line: works with 512 MB; RAM scales with render-thread-count | https://bluemap.bluecolored.de/community/RAM.html |
| OOM on small boxes when CLI runs alongside server (user had to use `-Xmx128m`) | https://github.com/BlueMap-Minecraft/BlueMap/issues/215 |
| v5.7 had to cap single hires-tile size — memory blowups on "overly complex worlds" | https://hangar.papermc.io/Blue/BlueMap/versions/5.7 |
| No render-rate limit: smallest unit is one tile render; only knobs are `render-thread-count`, `render-thread-priority`, `player-render-limit` | #215, #176, https://bluemap.bluecolored.de/wiki/getting-started/Configuration.html |
| CPU frequency drop on host from render threads raises MSPT even though rendering is off-thread | https://github.com/BlueMap-Minecraft/BlueMap/issues/176 |
| Historic poor multithread scaling (cache contention): 32 threads ≈ 1 thread (3.5 vs 3.9 tiles/s) on 0.10; after fix 1t≈2, 4t≈6, 32t≈24–44 tiles/s (2020, MC 1.16, hires 32×32 tiles) | https://github.com/BlueMap-Minecraft/BlueMap/issues/72 |
| Nether 7–8 → ~1.1 tiles/s (cave/roof heavy); advice: `min-y` on nether maps | https://github.com/BlueMap-Minecraft/BlueMap/issues/56 |
| Concurrency bug: unsynchronized `LowresLayer.save()` corrupts lowres PNGs with threads>1 (fixed 2026-07) — lowres aggregation is a shared-state hotspot | https://github.com/BlueMap-Minecraft/BlueMap/issues/821 |
| Startup slow for huge worlds: region enumeration in update-task constructor | https://github.com/BlueMap-Minecraft/BlueMap/issues/583 |
| Idle CPU (~16% of 8C) in Docker + Postgres after continuous-update feature; partly driver-related | https://github.com/BlueMap-Minecraft/BlueMap/issues/799 |
| Full updates re-triggered hourly (scheduling bug, 2026) | https://github.com/BlueMap-Minecraft/BlueMap/issues/869 |
| SQLite perf only improved in 5.28; wiki recommends `max-connections=1` for SQLite | v5.28 notes; https://bluemap.bluecolored.de/wiki/customization/Storages.html |
| Storage size: hires models already compressed; quality setting doesn't change file size; maps can exceed world size | https://github.com/BlueMap-Minecraft/BlueMap/issues/140 |
| Marker perf in webapp with ~8 MB marker file (deferred to webapp rewrite) | https://github.com/BlueMap-Minecraft/BlueMap/issues/600 |
| Integrated webserver DoS: unbounded chunked request body `new byte[size]` (master after 2026-07-03) | https://github.com/BlueMap-Minecraft/BlueMap/issues/828 |

Throughput numbers are scarce. Example world (#872): hires 144,804 tiles, lowres 878/45/5 for LOD 1/2/3,
~17.5k×22.5k blocks. https://github.com/BlueMap-Minecraft/BlueMap/issues/872
Third-party blog (unverified, low quality): BlueMap disk 8–15 GB for a 10k×10k world, roughly 3–5× squaremap.
https://mineguard.pro/en/blog/bluemap-vs-dynmap-vs-squaremap-comparison
**Recommendation:** build our own benchmark harness (tiles/s, peak RSS, bytes/tile) on fixed test worlds
and don't rely on published numbers.

## 3. Feature surface the port must match

Configs are **HOCON** (`.conf`). https://bluemap.bluecolored.de/wiki/getting-started/Configuration.html
- `core.conf`: accept-download, data, render-thread-count (≤0 = cores minus N), render-thread-priority,
  update-cooldown (60 s), full-update-interval (1440 min), scan-for-mod-resources, metrics, log.file/append.
  https://bluemap.bluecolored.de/wiki/configs/Core.html
- `plugin.conf`: live-player-markers, hidden-game-modes, hide-vanished/sneaking/below-sky-light/
  below-block-light/different-world, write-markers-interval, write-players-interval, skin-download,
  player-render-limit. https://bluemap.bluecolored.de/wiki/configs/Plugin.html
- `webserver.conf`: enabled, webroot, ip, port 8100, **sse-enabled** (live data over SSE), additional-headers,
  log.{file,append,format}. https://bluemap.bluecolored.de/wiki/configs/Webserver.html
- `webapp.conf`: enabled, webroot, update-settings-file, use-cookies, default-to-flat-view, start-location,
  min/max-zoom-distance, resolution-default, hires/lowres slider min/max/default, map-data-root,
  live-data-root, client-decompression, scripts, styles. https://bluemap.bluecolored.de/wiki/configs/Webapp.html
- `maps/*.conf`: world, dimension, name, sorting, start-pos, sky-color, void-color, sky-light, ambient-light,
  remove-caves-below-y (55), cave-detection-ocean-floor, cave-detection-uses-block-light, min-inhabited-time,
  render-mask (+ render-edges, edge-light-strength), enable-perspective/flat/free-flight-view, enable-hires,
  hires-tile-size 32, lowres-tile-size 500, lod-count 3, lod-factor 5, storage, ignore-missing-light-data,
  marker-sets, dimension-type. https://bluemap.bluecolored.de/wiki/configs/Maps.html ; masks:
  https://bluemap.bluecolored.de/wiki/customization/Masks.html
- `storages/*.conf`: FILE, SQL (MySQL, MariaDB, PostgreSQL, SQLite over JDBC). Per-map storage.
  https://bluemap.bluecolored.de/wiki/customization/Storages.html
  - Compression in source: **none, gzip (default), deflate, zstd, lz4** (`core/storage/compression/Compression.java`;
    wiki lists only 4).
- CLI flags: `-r` render, `-w` webserver, `-u` watch/update, `-c` config dir, `-h`. (Installation page above.)
- Tile formats: hires = binary mesh **`.prbm`** (`PRBMWriter.java` ↔ `webapp/.../PRBMLoader.js`), gzipped;
  lowres = PNG; `textures.json` gz. https://bluemap.bluecolored.de/wiki/webserver/ExternalWebserversFile.html
- External webservers: FILE needs `.gz` lookup + `Content-Encoding: gzip`, 204 for missing tiles
  (nginx `gzip_static always`, Apache rewrites, Caddy `try_files {path}.gz =204`); `/maps/*/live/*` proxied to
  :8100. SQL storage uses a shipped **`sql.php`** (PHP ≥ 7.4) that the server routes all misses to.
  https://bluemap.bluecolored.de/wiki/webserver/ExternalWebserversSQL.html ; reverse proxy:
  https://bluemap.bluecolored.de/wiki/webserver/ReverseProxy.html
- Markers: POI, HTML, Line, Shape, Extrude; static (map config `marker-sets`) or API; common fields type,
  position, label, sorting, listed, min/max-distance. https://bluemap.bluecolored.de/wiki/customization/Markers.html
- Mods/resource packs: auto-scan of mod jars + datapacks; `packs/` folder; `blockProperties.json`,
  `blockColors.json`, `defaultBlockstates.json` (needed for CLI). Runtime-generated mod resources aren't supported.
  https://bluemap.bluecolored.de/wiki/customization/Mods.html , https://bluemap.bluecolored.de/wiki/customization/ResourcePacks.html
- Server networks (BungeeCord/Velocity): https://bluemap.bluecolored.de/wiki/getting-started/ServerNetworks.html
- Commands/permissions: https://bluemap.bluecolored.de/wiki/getting-started/Commands.html
- API surface (BlueMapAPI classes): BlueMapAPI, BlueMapMap, BlueMapWorld, RenderManager, WebApp
  (register scripts/styles), AssetStorage, ContentTypeRegistry, markers/* (MarkerSet, POI/Html/Line/Shape/
  Extrude), plugin/* (PlayerIconFactory, SkinProvider, PlayerDisplayNameProvider), MarkerGson.
  https://github.com/BlueMap-Minecraft/BlueMapAPI
- **Native addons**: BlueMap loads addon jars itself (`common/addons/AddonLoader.java`, CombinedClassLoader) —
  a pure-Rust core cannot host these; we need either a JVM shim or drop this feature.
- Third-party addon ecosystem is large and Java-API based. Registry: https://bluemap.bluecolored.de/3rdPartySupport.html
- Known incompatibles: JustEnoughIDs, NotEnoughIDs, OpenCubicChunks, SlimeWorldManager. https://bluemap.bluecolored.de/wiki/FAQ.html

## 4. Prior art

| Project | Lang / license | Relevance |
|---|---|---|
| **MinedMap** (neocturne) | Rust, MIT | 2D top-down + Leaflet. MC 1.8–26.1. Claims full map of a 3 GB save in <5 min **single-threaded**, <100 MB RAM; incremental by region mtime; `-j N`. Deps: fastnbt, flate2 with **zlib-rs**, image (png/webp), rayon, lru, notify, postcard, zstd, jemalloc. https://github.com/neocturne/MinedMap |
| **fastnbt / fastanvil** | Rust, MIT | serde NBT plus region reader/renderer; fastanvil supports ≥1.13 (1.12 flaky); has WASM demo; LZ4 via `lz4-java-wrc`. fastnbt 2.6.3 (2026-08); fastanvil 0.32 (2025-08). https://github.com/owengage/fastnbt |
| **simdnbt** (azalea) | Rust, MIT | Fastest NBT reader: borrow mode 4.3 GiB/s vs fastnbt 161 MiB/s, valence_nbt 276 MiB/s on complex_player.dat. 0.10.0. https://github.com/azalea-rs/simdnbt |
| valence | Rust, MIT | Server framework; `valence_anvil`/`valence_nbt` stale since 2023-10 (crates.io). https://github.com/valence-rs/valence |
| azalea | Rust, MIT | Bot/client crates, active 2026-10. https://github.com/azalea-rs/azalea |
| mcaselector | Java, MIT | Mature MCA chunk tooling, very active; good format reference. https://github.com/Querz/mcaselector |
| Chunky (path tracer) | Java, **GPL-3** | Don't copy code. https://github.com/chunky-dev/chunky |
| Dynmap | Java, Apache-2 | 2D/iso tiles, millions of small files. https://github.com/webbukkit/dynmap |
| squaremap | Java, MIT | Vanilla-style 2D, "ultra fast render times"; default ~32 chunks/s rate cap (blog, unverified). https://github.com/jpenilla/squaremap |
| Pl3xMap | Java, MIT | Like squaremap. https://github.com/granny/Pl3xMap |
| Mapcrafter | C++ | Older isometric 3D renderer, high-performance reference. https://mapcrafter.readthedocs.io/ |
| uNmINeD | closed source (.NET) | 2D viewer/renderer; no reusable code. https://unmined.net/ |

No mature Rust **3D web** map renderer found. BlueMap-style 3D meshes in Rust looks like open ground.

## 5. Crate choices (crates.io, 2026-10-06)

| Need | Pick | Notes |
|---|---|---|
| NBT | **simdnbt** 0.10 (borrow) for hot chunk path; fastnbt 2.6 (serde) for configs/level.dat | simdnbt 25× fastnbt in its bench |
| Region | own reader (~200 LOC) over `memmap2` 0.9 / pread; fastanvil as reference | must handle types 1/2/3/4/127, `.mcc` |
| zlib | **flate2 1.1 + `zlib-rs` feature** (zlib-rs 0.6.8). Alternative: `libdeflater` 1.26 (whole-buffer, C) since chunk sizes are known | zlib-rs beats zlib-ng on decompress for 1–65 KB inputs. https://trifectatech.org/blog/zlib-rs-is-faster-than-c/ ; flate2 defaults to miniz_oxide (slower) |
| LZ4 (MC) | `lz4_flex` 0.14 block decoder + own LZ4BlockOutputStream framing parser, or `lz4-java-wrc` 0.2 | MC uses lz4-java's `LZ4Block` format, **not** LZ4 frame. https://minecraft.wiki/w/Region_file_format |
| zstd | `zstd` 0.14 | BlueMap storage option |
| PNG encode | `png` 0.18 (uses **fdeflate** fast mode) or `image` 0.25; `zune-png` decode | fdeflate fast mode encodes far faster than libpng. https://lib.rs/fdeflate |
| HTTP | **axum** 0.8 + hyper 1.x + tower-http 0.7 (static files, precompressed gz, compression) | SSE built into axum |
| SQL | **sqlx** 0.9 (Postgres/MySQL/SQLite async) or rusqlite 0.40 + tokio-postgres 0.7 + mysql_async 0.37 | sqlx gives one API over 3 dialects |
| Parallelism | rayon 1.12 | |
| Allocator | mimalloc 0.1.52 / tikv-jemallocator 0.7 | MinedMap defaults to jemalloc |
| JNI | jni 0.22.4 (2026-03), j4rs 0.25 | jni-rs is the de facto standard |
| HOCON | `hocon` 0.9 — **stale since 2022** | Risk: we may need our own HOCON parser or a vendored fork |

## 6. Format drift to track

Region file (https://minecraft.wiki/w/Region_file_format):
- 8 KiB header (offsets + timestamps), 4 KiB sectors, ≤255 sectors/chunk; oversize → `c.X.Z.mcc` with type|128.
- Compression ids: 1 gzip, 2 zlib (default), 3 none, **4 LZ4 (24w04a / 1.20.5)**, **127 custom namespaced (24w05a)**.
- `region-file-compression` server.properties: `deflate` (default) | `lz4`; existing chunks aren't recompressed,
  so **one region file can mix types**. https://minecraft.wiki/w/Java_Edition_24w04a

Chunk NBT (https://minecraft.wiki/w/Chunk_format):
- 1.13 flattening (palette + BlockStates); 1.16 packed longs no longer span elements; 1.18 (`21w43a`): `Level`
  removed, `sections[].block_states{palette,data}`, paletted biomes 4×4×4, `yPos`, negative Y.
  https://minecraft.net/ja-jp/article/minecraft-snapshot-21w43a
- **26.3 snapshot 7**: palette `Name`→`id`, `Properties`→`properties`; properties omitted for default state;
  **palette may be a list of strings**.
- **26.3 snapshot 10**: statuses noise/surface/carvers → `minecraft:terrain` (affects "is chunk complete" checks).
- **26.4 snapshot 1 (upcoming)**: **biomes stored per block, not per 4×4×4 cell**; `Status`→`status`.
- Data versions: 1.13=1519, 1.16=2566, 1.18=2860, 1.20.5=3837, 1.21=3953, 1.21.4=4189, 1.21.11=4671,
  26.1=4786, 26.2=4903, 26.3=5023, 26.4-snapshot-2=5120. https://minecraft.wiki/w/Data_version
  → Dispatch chunk decoders on DataVersion ranges (BlueMap has Chunk_1_13/1_15/1_16/1_18 classes).

World layout:
- **26.1 restructure**: dimensions live in `dimensions/<ns>/<path>/` (overworld at `dimensions/minecraft/overworld/`,
  not world root; `DIM-1`/`DIM1` gone); players moved under `players/`; `data/` namespaced.
  https://papermc.io/news/26-1 , https://feedback.minecraft.net/hc/en-us/articles/43431470235021-Minecraft-Java-Edition-26-1-Snapshot-6
- Paper adopted the vanilla layout in 26.1; dimension keys may contain `/` (nested dirs).
  https://github.com/BlueMap-Minecraft/BlueMap/issues/820
- Versioning: year-based since 26.1 (released 2026-03-24); 26.3 released 2026-09-15.
  https://minecraft.wiki/w/Java_Edition_26.3

## 7. JVM interop (embedding Rust in Paper/Fabric)

- **Java requirement**: 1.20.5–1.21.x need Java 21; **26.1+ requires Java 25** (LTS).
  https://minecraft.wiki/w/Java_Edition_26.1-snapshot-1 , https://minecraft.wiki/w/Java_Edition_26.3
  → FFM (final in Java 22, JEP 454) is usable on every 26.x server; 1.20.5–1.21.x on Java 21 have FFM as
  preview only (needs `--enable-preview`), so supporting them requires JNI.
- **JEP 472 (JDK 24+)**: JNI `System.loadLibrary` and FFM downcalls are "restricted"; default is **one warning
  per module**, and a future JDK will **deny by default**. Fix: `--enable-native-access=ALL-UNNAMED` or a
  `Enable-Native-Access` manifest attribute, which only applies to an executable JAR (plugins/mods can't set it).
  https://openjdk.org/jeps/472
  → Server owners will see a warning. Document the JVM flag and plan for a future deny-by-default JDK.
- jni-rs 0.22.4 is mature (≈200M downloads). https://crates.io/crates/jni
- Precedents shipping Rust natives in MC:
  - **Oxidizium** (Fabric, 1.14.3–1.21.11): Rust replacements, backends Panama / Nalim / Membrane; claims Nalim is
    about 3× faster than Panama for tiny calls. https://modrinth.com/mod/oxidizium
  - FerricOxide (Rust JNI webview bridge). https://www.curseforge.com/minecraft/mc-mods/ferric-oxide
  - Yog runtime (Fabric↔Rust over JNI + C ABI). https://docs.rs/crate/yog-runtime/0.7.0
  - BlackboxMC (Spigot plugins in native langs, Rust JNI lib). https://minecraftbible.com/plugins/project/blackboxmc
  - Velocity natives (C, libdeflate) are the long-standing precedent for per-OS natives inside a jar.
- Design implication: BlueMap's render path is coarse-grained (whole tiles), so per-call FFI overhead doesn't
  matter. Prefer a narrow C ABI (render region/tile, push world-change events, query state). Also consider an
  **out-of-process** Rust daemon plus a thin Java plugin for events, live players, and the API. That avoids native
  loading in the server JVM and the JEP 472 warning, and keeps Rust memory outside the server's `-Xmx`.

## 8. License

- BlueMap and BlueMapAPI are **MIT** ("Copyright (c) Blue <https://www.bluecolored.de> / contributors").
  https://github.com/BlueMap-Minecraft/BlueMap/blob/master/LICENSE
  → Porting logic, reusing the webapp (Vue/three.js), PRBM format, `sql.php`, and default resource JSONs is
  allowed. Requirement: keep the MIT notice and copyright in copies/substantial portions (bundle the original
  LICENSE with the reused webapp).
- Minecraft client assets are **not** redistributable. BlueMap downloads the client jar only after
  `accept-download=true` (EULA gate), and the port must keep that gate. https://bluemap.bluecolored.de/wiki/configs/Core.html
- Avoid copying from GPL code (Chunky). Dynmap is Apache-2 (NOTICE needed if reused). MinedMap, fastnbt,
  simdnbt, squaremap, and Pl3xMap are MIT.
- The "BlueMap" name/branding is not covered by MIT. Use a distinct project name and state it's a port.
