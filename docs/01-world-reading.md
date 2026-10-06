# 01 — World data reading (BlueMap Java → Rust)

Paths are relative to the BlueMap checkout. `W = core/src/main/java/de/bluecolored/bluemap/core/world`.

## 0. Dependencies

- NBT: **BlueNBT 3.5.1** (`gradle/libs.versions.toml:16`, `core/build.gradle.kts:9`). It is the author's own library: a reflection-based, Gson-style **streaming** deserializer (`NBTReader` pull parser plus `TypeDeserializer`/`TypeResolver`/`@NBTName`). Chunk NBT is mapped straight onto POJOs, with **no intermediate tag tree**.
- Compression: JDK `GZIPInputStream`/`InflaterInputStream`, `lz4-java` 1.10.1 (`LZ4BlockInputStream`), aircompressor (zstd, used only for BlueMap's own storage, not for world reading). See `core/storage/compression/Compression.java:45-49`.
- Caching: Caffeine 3.3.0 (`Caches.java:34-53`). The string interner is also Caffeine's (`util/StringUtil.java:36`).
- `core/util/nbt/*` (PalettedArrayAdapter, RegistryAdapter, LenientListAdapter) is **not world reading**. Only `map/renderstate` (BlueMap's own tile-state files) uses it. Ignore it for this area.

## 1. Region files

- **Only Anvil `.mca` is supported.** `RegionType.REGISTRY` contains just `MCA` (`W/mca/region/RegionType.java:44-49`). There is no `.linear`, no `.mcr`, and no pre-Anvil support.
- Filename regex `^r\.(-?\d+)\.(-?\d+)\.mca$` (`MCARegion.java:47`). Region coordinates beyond ±100000 are rejected (`RegionType.java:122-127`). Zero-byte files are skipped in `listRegions` (`ChunkGrid.java:127`).
- Header layout: 4 KiB of locations (3-byte sector offset + 1-byte sector count), then 4 KiB of big-endian i32 timestamps (`MCARegion.java:86-96`, `139-143`).
- Chunk payload: a 4-byte length, which is **ignored** (they read `sectorCount*4096` bytes and pass `size-5` to the decompressor), then a 1-byte compression id (`MCARegion.java:217-235`).
- Compression map (`MCARegion.java:49-56`): `0→NONE` (non-standard leniency), `1→GZIP`, `2→DEFLATE (zlib)`, `3→NONE`, `4→LZ4`. **127 (custom algorithm, MC 1.20.5+) is unsupported** and throws "Unknown chunk compression-id".
- LZ4 is lz4-java's `LZ4BlockInputStream` framing, which is what Mojang uses. **This is not the LZ4 frame format.**
- External oversized chunks: if `id > 127`, the code subtracts 128 and reads `c.<cx>.<cz>.mcc` from the same folder (`MCARegion.java:223-229`).
- **How a region is opened.** The `MCARegion` object holds only the `Path` plus a cached fingerprint. Each `loadChunk` call does the following:
  - `Files.exists` + `Files.size`
  - `FileChannel.open`
  - a 4-byte header read and a `new byte[size]` allocation
  - a read, then close (`MCARegion.java:79-109`)

  That is **one open/close syscall pair per chunk**. Reads are lazy (one chunk at a time) and there is no mmap.
- `iterateAllChunks` (`MCARegion.java:112-174`) reads the 8 KiB header once and reuses one buffer. A consumer `filter(x,z,timestamp)` decides whether each chunk gets decoded. This is the bulk/preload path.
- **Change detection uses region-header timestamps.** `WorldRegionUpdateTask.java:108-115` collects the per-chunk timestamps as "chunk hashes". `Region.fingerprint()` is a 31-hash over the 8 KiB header, cached for 10 s (`MCARegion.java:58,177-205`). `MapUpdateService.java:202` polls it.
- A file watcher (`MCAWorldRegionWatchService.java`) uses the JDK `WatchService` on `region/` and maps filenames to region coordinates.

## 2. Chunk loading and version handling

The dispatcher is `W/mca/chunk/MCAChunkLoader.java:60-92`. Version classes are selected by `DataVersion >= threshold`:

| Class | min DataVersion | MC | Root | Block storage | Biomes |
|---|---|---|---|---|---|
| `Chunk_1_18` | 2844 (21w43a) | 1.18+ | flat root | `sections[].block_states{palette,data}`, **padded** longs | per-section `biomes{palette:[str],data}` 4×4×4, padded |
| `Chunk_1_16` | 2500 | 1.16–1.17 | `Level` | `Sections[].Palette` + `BlockStates`, **padded** | `Level.Biomes` int[1024] (4×4×4 cells, legacy numeric ids) |
| `Chunk_1_15` | 2200 | 1.15 | `Level` | extends 1_13: **non-padded (spanning)** | int[1024] 3-D, with y clamping (`Chunk_1_15.java:37-48`) |
| `Chunk_1_13` | 0 | 1.13–1.14 | `Level` | non-padded spanning `BlockStates` | int[256] 2-D (`Chunk_1_13.java:160-167`) |

- **Pre-1.13 numeric block worlds are not supported.** They fall into `Chunk_1_13`, which has no `Palette`, so every block reads as air.
- **Speculative parse.** The loader parses with `lastUsedLoader` first. If the resulting `DataVersion` picks a different loader, it **re-decompresses and re-parses** (`MCAChunkLoader.java:74-89`). `lastUsedLoader` is a shared, non-volatile field: a benign race, but mixed-version worlds pay for a double parse.
- NBT field names come from `NamingStrategy.lowerCaseWithDelimiter("_")` (`MCAUtil.java:51`). So the Java field `blockStates` maps to `block_states` and `blockEntities` to `block_entities`. Legacy names are set explicitly with `@NBTName`.
- **Padded decode** (1.16+), `PackedIntArrayAccess.java`:
  - `bitsPerElement = max(longs*64/4096, 1)` is **inferred from the array length**, not from the palette size (`:105-107`).
  - `elementsPerLong = 64/bits`. Division uses a magic-multiply table (`:30-97`, `:130-133`).
  - Indexes past the end return 0.
- **Spanning decode** (1.13–1.15): `MCAUtil.getValueFromLongStream` (`MCAUtil.java:72-87`) with `bitsPerBlock = longs.length >> 6` (`Chunk_1_13.java:253`).
- Block index is `(y&15)<<8 | (z&15)<<4 | x&15`. Biome index for 1.18 is `(y&12)<<2 | z&12 | (x&12)>>2` (`Chunk_1_18.java:253,266`).
- Biome bit width for 1.18 is `max(ceilLog2(paletteLen),1)` (`Chunk_1_18.java:243`). Biome keys are resolved to `Biome` objects **once per section at load** through `DataPack.getBiome(Key)` (`:235-240`). Unknown keys become `Biome.DEFAULT`.
- Legacy int biome ids map through a 170-entry table (`W/mca/chunk/LegacyBiomes.java:34+`, wired in `DataPack.java:138,149`). This lookup happens **per call** in 1.13–1.17 `getBiome`.
- Out-of-range palette index: returns `BlockState.MISSING`/`Biome.DEFAULT` and logs a no-flood warning. Palette size 1 short-circuits; size 0 means air (`Chunk_1_18.java:249-260`).
- **Light**:
  - Each section has `BlockLight` and `SkyLight` nibble arrays (2048 B). The low nibble is the even index (`MCAUtil.getByteHalf`, `Chunk_1_18.java:275-286`).
  - Light data counts as present only if `Status == minecraft:full`. Legacy versions also accept `fullchunk`/`postprocessed` (`Chunk_1_13.java:76-79`).
  - With no light data the result is `(skyLight = dim.hasSkylight ? 15 : 0, block = 0)`. A missing section below `sectionMin` gives (0,0); a missing section above gives (sky, 0) (`Chunk_1_18.java:164-172`).
  - **Light is used for rendering**: face shading in `hires/block/ResourceModelRenderer.java:189-190`, liquids, and the top-block light in `BlockRenderPass.java:82`.
  - It also gates rendering: chunks without light produce `TileState.MISSING_LIGHT` unless `ignoreMissingLightData` is set (`WorldRegionUpdateTask.java:362-365`).
- **Heightmaps**: only `WORLD_SURFACE` and `OCEAN_FLOOR`.
  - 1.16+: padded, `bits = ceilLog2(dimHeight+1)`, value `+ minY`, validity checked through `isCorrectSize(256)` (`Chunk_1_18.java:80-87,190-202`).
  - 1.13–1.15: spanning, 9 bits, valid if at least 36 longs (`Chunk_1_13.java:88,197-215`).
  - `OCEAN_FLOOR` drives cave removal (`ExtendedBlock.isRemoveIfCave`, `ExtendedBlock.java:159-171`). `WORLD_SURFACE` is exposed on `Chunk` but **not used by the renderer** (grep finds no caller outside `world/`).
- `getMinY`/`getMaxY` come from the actual section range (`sectionMin*16 .. sectionMax*16+15`). That includes the light-only padding sections (`Chunk_1_18.java:174-182`). The renderer clamps each column to it (`BlockRenderPass.java:69-71`).
- Also read: `InhabitedTime` (used for the `minInhabitedTime` filter) and `Status` (`generated = status != empty`).
- **Block entities**:
  - Sources are `block_entities` (1.18) and `Level.TileEntities`. A `LenientBlockEntityArrayDeserializer` turns a wrong tag type into an empty list (`data/LenientBlockEntityArrayDeserializer.java:46-52`).
  - Type resolution goes through `BlockEntityTypeResolver` on `id`. Typed classes exist only for sign, hanging_sign, skull and banner (`blockentity/BlockEntityType.java:36-46`). Everything else becomes a base `MCABlockEntity {id,x,y,z,keepPacked}`, and parse errors fall back to that base.
  - Storage is a `HashMap<Long,BlockEntity>` keyed by `y<<8|(x&15)<<4|z&15` (boxed Long, `Chunk_1_18.java:122-129,205-207`).
- **Entities** are read only from the separate `entities/` region folder (1.17+) through a second `ChunkGrid<MCAEntityChunk>` (`MCAWorld.java:83`). Fields read: `Entities[]` with `id, UUID, CustomName, Pos, Motion, Rotation` (`entity/MCAEntity.java:44-50`). **Pre-1.17 in-chunk `Level.Entities` are ignored.** `iterateEntities` walks the chunks overlapping a rectangle and filters by floored position (`MCAWorld.java:141-164`).

## 3. Abstractions and the renderer-facing API

- `World` (`W/World.java`): `getChunk(cx,cz)`, `getChunkAtBlock(x,z)`, `getRegion`, `listRegions` (uncached directory listing), `preloadRegionChunks(rx,rz,filter)`, `invalidateChunkCache(...)`, `iterateEntities`, `getDimensionType`. The contract says implementations must be thread-safe.
- `Chunk` (`W/Chunk.java:32-85`): `getBlockState(x,y,z)`, `getLightData(x,y,z,target)`, `getBiome`, `getMin/MaxY`, heightmaps, `getBlockEntity`, `isGenerated`, `hasLightData`, `getInhabitedTime`. The `EMPTY_CHUNK` and `ERRORED_CHUNK` sentinels are compared by identity.
- **Hot path**: `BlockRenderPass.render` loops x, z, then y descending, calling `BlockNeighborhood.set(x,y,z)` (`BlockRenderPass.java:57-95`).
  - `BlockNeighborhood` is an 8×8×8 ring of `ExtendedBlock` → `Block` wrappers indexed by `coord & 7` (`block/BlockNeighborhood.java:33-84`). The model renderers fetch neighbors through it.
  - `Block` lazily caches chunk, state, light, biome and block entity, and resets the chunk only when x or z changes (`block/Block.java:52-105`).
  - Each of the 512 neighborhood slots holds its own `Block` with its own chunk ref, so a chunk lookup (Caffeine `get` with a `Vector2i` key) happens per slot whenever x or z changes.
  - `ExtendedBlock.getProperties()` does a Caffeine lookup keyed by `BlockState` on every position change (`ExtendedBlock.java:133-136`, `ResourcePack.java:356-357`).
- **BlockState** (`W/BlockState.java`):
  - Fields: `Key id` (interned namespace/value strings, `util/Key.java:47-55`), `Map<String,String> properties` (a LinkedHashMap from the deserializer), a sorted `Property[]` with interned key and value strings, a cached hash, and the flags `isAir/isWater/isWaterlogged`. Liquid level and power are parsed lazily.
  - **States are not interned.** Every palette entry of every loaded section allocates a new `BlockState`, `LinkedHashMap`, `Property[]` and `Key` (`data/BlockStateDeserializer.java:52-87`).
  - Equality compares id plus the sorted property array.
  - A palette entry may also be a bare string or `{"": id}`. Those resolve to a configured default state through `DataPack.getDefaultBlockState` (`defaultBlockstates.json`, `BlockStateDeserializer.java:53-54,72-75,89-96`).
- `LightData` is a mutable `(sky, block)` holder that is reused to avoid allocation.
- `Biome` is an interface backed by datapack JSON (temperature, downfall, water/foliage/grass colors, grass modifier).
- `DimensionType` (`W/DimensionType.java`): `hasSkylight, hasCeiling, ambientLight, minY, height, fixedTime, coordinateScale`. Built-ins are overworld (-64/384), nether, end and overworld_caves.

## 4. Caching, memory, threading

- One `ChunkGrid` per world per kind (blocks and entities), `W/mca/ChunkGrid.java:60-70`:
  - **regionCache**: soft values, expires 10 min after write and 1 min after access. It caches only the light `MCARegion` objects.
  - **chunkCache**: soft values, `maximumSize(10240)` ("10 regions worth"), same expiry. Async maintenance runs on `BlueMap.THREAD_POOL`.
- **Preload.** `WorldRegionUpdateTask.java:144-145` preloads a whole region into the cache if at least 75% of its tiles need rendering. It first invalidates every chunk listed in the region header (`:114`).
- **Load failure.** Loading retries **3 times with `Thread.sleep(1000)`** on the render thread (`ChunkGrid.java:168-196`), then returns `ERRORED_CHUNK`.
- **Memory.** A full 1.18 chunk with 24 sections holds:
  - about 2×2 KiB of light per section, so around 100 KB of light per chunk;
  - block `long[]` data of 4096×bits/8 bytes per section, plus palette objects;
  - Java object headers on top.

  At 10240 chunks the cache can reach roughly 1–2 GB, and only the soft references keep it bounded. Expect GC pressure.
- **Threading.** Chunks are immutable after construction and Caffeine handles concurrency. Benign data races exist in the static `Vector2iCache` (`util/Vector2iCache.java`, unsynchronized but storing immutable vectors), `MCAChunkLoader.lastUsedLoader`, and `MCARegion.lastFingerprint`. `Block`/`BlockNeighborhood` are per-thread scratch objects.

## 5. Dimensions, world folder, level.dat, datapacks

- Each map config gives `world` (a path) plus `dimension` and an optional `dimensionType` key (`common/config/MapConfig.java:59-60`). There is no automatic world discovery inside `core`.
- The single loader is `WorldLoaderType.ANVIL → MCAWorld::load` (`W/WorldLoaderType.java:37`).
- Dimension folder (`MCAWorld.java:174-191`):
  1. New layout: `<world>/dimensions/<ns>/<path>`.
  2. Legacy layout: the overworld is `<world>`, the nether `DIM-1`, the end `DIM1`, and others `dimensions/<ns>/<path>`. This layout is used only if a `region/` folder exists there.

  Inside the dimension folder: `region/` and `entities/`.
- Dimension type resolution (`MCAWorld.java:193-224`):
  1. An explicit config key.
  2. `<dimFolder>/data/minecraft/world_gen_settings.dat`, then `<world>/data/minecraft/world_gen_settings.dat`, reading `data.dimensions[<dim>].type`.
  3. `level.dat`, reading `Data.WorldGenSettings.dimensions[<dim>].type`.
  4. A vanilla built-in, else OVERWORLD with a warning.

  `type` is either an inline compound or a string reference into the datapack (`data/DimensionTypeDeserializer.java:47-63`). All `.dat` files are gzip.
- Datapacks: the `<world>/datapacks/*` entries are added (`W/WorldLoader.java:50-58`). `DataPack` loads `data/*/dimension_type/**.json`, `data/*/worldgen/biome/**.json` and `defaultBlockstates.json` (`resources/pack/datapack/DataPack.java:89-130`).
- `LevelData` also exposes `LevelName` and spawn (`data/LevelData.java:44-93`).

## 6. Java hot spots and waste (what to avoid)

1. **One file open per chunk.** `FileChannel.open/close` plus `Files.exists/size` (3–4 syscalls) and a fresh `byte[]` on every cache miss (`MCARegion.java:80-103`).
2. **Stream-based decompression** through `BufferedCompression`, plus a possible **double decompress and parse** on a version guess miss.
3. **Object-heavy palette materialization.** One `BlockState` per palette entry per section per load, with a LinkedHashMap, a sorted array, string interning through Caffeine, and `Key.parse` allocation.
4. **Per-block property lookups.** Every `getProperties()` call after a position change does a hash plus `Property[]` equality (non-identity) lookup in a Caffeine cache to get `BlockProperties`.
5. **Hash-map chunk lookups.** Chunk lookup is a Caffeine hash lookup per neighborhood slot per x/z change, instead of a direct array index into a region working set.
6. **Boxing.** `HashMap<Long,BlockEntity>` boxes its keys. `Entity[]`/`BlockEntity` are POJOs per entry.
7. **Allocated arrays.** Each `long[]` and `byte[]` light array is copied into a fresh Java array by the reflection deserializer. Light arrays are kept even when uniform.
8. **Per-call legacy biome mapping.** 1.13–1.17 `getBiome` maps legacy ids through the DataPack on every call.
9. **Memory-pressure eviction.** Soft-value caches evict by GC pressure, which makes performance unpredictable.
10. **Blocking retries.** Failed loads sleep the render thread for 2 s in total.
11. **What does not need fixing.** NBT is not tree-materialized, and the packed arrays are read in place with magic division. Both are already fairly lean.

## 7. Rust plan

### Crates
- **NBT**: `simdnbt` in borrow mode, the fastest option and zero-copy. It exposes `&[u8]` big-endian long and byte arrays plus random access to compounds. This lets you peek `DataVersion` first and avoid the double parse. `fastnbt` with serde (`fastnbt::borrow::LongArray`) is the simpler fallback and has good derive ergonomics for `level.dat` and entities. Recommendation: simdnbt for the chunk hot path, fastnbt/serde for cold files (`level.dat`, `world_gen_settings.dat`, entities).
- **zlib/gzip**: `flate2` with the `zlib-rs` backend (pure Rust, near zlib-ng speed). Or use `libdeflater` for whole-buffer decompression into a reused, growing `Vec`. That works for gzip because ISIZE is in the trailer; for zlib, guess the size and grow.
- **LZ4**: `lz4_flex::block::decompress_into` behind a **hand-written lz4-java `LZ4Block` frame parser**:
  - Each block is: magic `"LZ4Block"`, token (`method` in the high nibble, `0x10` raw / `0x20` lz4; compression level in the low nibble), compressed length (u32 LE), decompressed length (u32 LE), and a checksum (xxhash32 with seed `0x9747B28C`, masked).
  - Loop over blocks until a terminating zero-length block.
  - `lz4_flex::frame` will **not** read this format.
- **Caching**: `quick_cache` or `moka` (sync) only if a global cache is still needed. A per-task working set is preferred; see below.
- **I/O**: `std::os::unix::fs::FileExt::read_at` / `std::os::windows::fs::FileExt::seek_read` on a file handle cached per region. Use `memmap2` only with care: the server rewrites region files live, so a truncated mapping can raise SIGBUS on Linux. Prefer pread plus a per-thread buffer pool.
- Also: `rustc-hash`/`ahash` for interning, `dashmap` or a sharded `parking_lot::RwLock<HashMap>` for the global state registry, and `notify` for the region watcher.

### Data layout
- **Global block-state interning.** A `BlockStateRegistry` maps `(name, sorted props)` to a `u32` id, with a dense `Vec<BlockStateInfo>`. That vector holds the flags `is_air/is_water/waterlogged/level/power` and, after the resource pack loads, a dense `Vec<BlockProperties>` indexed by id. This removes Java waste items 3 and 4.
  - Speed up interning with a per-thread `FxHashMap<u64 /* hash of raw palette-entry NBT bytes */, u32>` fast path, so most palette entries never allocate strings.
- Biomes get the same treatment: a `u16` biome id is resolved once per palette entry. Legacy numeric ids map through a static table at load.
- **Section**: `enum Blocks { Single(u32), Packed { palette: SmallVec<[u32;16]>, bits: u8, data: Box<[u64]> } }`.
  - Keep the data packed (converted to native-endian once, or read with `u64::from_be_bytes` straight from the decompressed buffer if the buffer's lifetime is kept). Precompute `elems_per_long` and a magic-division shift, or simply fully unpack to `[u16; 4096]` (8 KiB).
  - Unpacking makes `get` a single load. It costs about 200 KB per full chunk, which is fine for a bounded working set and too much for a 10k-chunk cache. Benchmark both.
- **Light**: `Option<Box<[u8; 2048]>>` per kind. Collapse all-0 and all-15 arrays to an enum tag. Also store a chunk-level `has_light` flag.
- **Biomes**: `[u16; 64]` per section (1.18+), unpacked eagerly because it is tiny.
- Heightmaps: unpack to `[i16; 256]` at load (only `OCEAN_FLOOR` is needed for rendering).
- Block entities: `Vec` sorted by packed `(y,x,z)` key, or a `HashMap<u32,..>`; most chunks have few. Keep only the sign/skull/banner payloads typed and store the rest as id plus position.

### Working set instead of a global LRU
- Renders are scheduled per region (`WorldRegionUpdateTask`). Load the region's 32×32 chunks plus a 1-chunk border into a `Vec<Option<Arc<Chunk>>>` arena indexed by `(cx - min, cz - min)`. Neighbor access then becomes an array index with no hashing.
- Share border chunks between concurrently rendering regions with a small concurrent cache (for example `quick_cache` with a weight on bytes), or simply re-decode them, since chunk decode is cheap in Rust.
- Replace the 1 s retry sleeps with a single retry and no sleep. Report errored chunks to the scheduler instead.

### Version gotchas to replicate
- Dispatch on `DataVersion`: ≥2844 flat root, `sections`, `block_states`, `biomes`, `block_entities`, `Heightmaps`. Below that: `Level` wrapper, `Sections`, `Palette`, `BlockStates`, `Biomes` int array, `TileEntities`.
- Padded longs for ≥2500, spanning longs below. Block bits come from **array length**, not palette size; biome bits come from `ceil_log2(palette_len)` with a minimum of 1.
- Heightmap bits: `ceil_log2(height+1)` padded for ≥2500, 9-bit spanning below. Add `min_y` only for the 1.18 path.
- Biome ints: 256 entries (1.13–1.14, 2-D) vs 1024 (1.15–1.17, 3-D with the y clamp in `Chunk_1_15.java:42-44`).
- Status strings: `empty`, `full`, plus the legacy `fullchunk`/`postprocessed`. The `minecraft:` namespace may be omitted.
- Region quirks: compression id 0 is treated as none, `+128` means an external `.mcc` file, and 127 means custom. Consider supporting 127 (namespaced algorithm string) and possibly `.linear` as extensions.
- Sections can extend past the dimension's min/max (light padding). Out-of-range palette indices must not panic.
- **Pre-1.13 worlds stay unsupported** for parity. Make that explicit instead of silently rendering air.
- Dimension folder: new `dimensions/<ns>/<path>` layout first, then legacy `DIM-1`/`DIM1`. Dimension type lookup goes `world_gen_settings.dat` → `level.dat` → built-in. All `.dat` files are gzip.
