# 10 — Performance audit: BlueMap Java render pipeline

This is a static read of the source; nothing was built. Paths are relative to the BlueMap checkout:
- `C/` = `core/src/main/java/de/bluecolored/bluemap/core/`
- `CM/` = `common/src/main/java/de/bluecolored/bluemap/common/`

Items already covered in `01-world-reading.md` §6 and `03-rendering.md` §8 are only referenced here, not repeated: per-file open, double parse, non-interned `BlockState`, per-block variant matching, 4–5 transform sweeps, 75-sample biome blend, and others.

The profile evidence came from the coordinator, for a 22-thread render:
- The JVM averaged about 17% machine CPU, roughly 4 of 22 cores.
- Monitor waits: `RenderManager.doWork` (avg 2 s), `hasMoreWork` (avg 2.5 s), `BmMap.save` (avg 15 s, max 40 s), and `MapTileState.set`.
- CPU: Caffeine about 20%, `BufferedOutputStream.growIfNeeded` 10%, `BlockNeighborhood.getBlock` 9%, `ChunkGrid.getChunk` 7.6%.
- Allocation: the `BlockStateModelRenderer` lambda 24%, and `int[]` 35%, of which `IntegerInterleavedRaster.getDataElements` is 28%.
- Each lowres PNG was written 4–7 times per render.

Every one of these has a source-level cause below.

## Measured (2026-10-06, BlueMap 5.27, JDK 25, `structures` fixture: 31 regions, 22-thread machine)

Scripts are in `docs/perf-exp/`: `java_bench.py` runs a clean force-render, and `agg.py`, `incl.py` and `park.py` aggregate JFR stacks. Each configuration was run once, on a machine that was not idle.

| threads | wall | CPU | avg busy cores | peak RSS |
|---|---|---|---|---|
| 22 | 307 s | 1108 s | 3.6 | 1067 MB |
| 4 | 285 s | 882 s | 3.1 | 858 MB |

- **Adding threads buys nothing past about 4.** This is the #1 convoy below.
- Render-thread monitor-enter time in JFR was 1307 thread-s:
  - 983 s at `RenderManager.doWork` `synchronized(renderTasks)`;
  - 120 s at `BmMap.save` called from `complete()`;
  - about 150 s at `hasMoreWork`.
- Inclusive CPU by category (first matching frame from the leaf):

  | Category | Share |
  |---|---|
  | Caffeine lookups | 38% |
  | Model render | 18% |
  | Neighbourhood/block access | 13% |
  | PRBM write | 11% |
  | `getChunk` | 7.6% |
  | Lowres | 6.7% |
  | Region read/decompress/NBT | **1.7%** |

  World I/O is not the bottleneck.
- G1 live set after GC was about 380–440 MB, and the heap peaked around 840 MB.
- The output is deterministic: md5 matches across runs.

## Ranked summary

| # | Finding | Where | Est. impact | Kind |
|---|---|---|---|---|
| 1 | A monitor chain lets one map save stall **every** render thread | `CM/rendermanager/RenderManager.java:306-318`, `CombinedRenderTask.java:52-67`, `WorldRegionUpdateTask.java:160-191,239-264`, `C/map/BmMap.java:159-186` | **4–5× wall clock**; explains the 17% CPU | contention |
| 2 | `BmMap.save` is slow and amplified: lowres PNG rewrites, per-pixel `int[]` allocation, state saves under the lock that render threads need | `C/map/lowres/LowresLayer.java:104-228`, `LowresTile.java:64-100`, `C/map/renderstate/CellStorage.java:84-142` | It is the 15 s that #1 waits on. 4–7× redundant PNG encodes. 28% of allocation | I/O, alloc |
| 3 | Region `init()` and the full-region preload (block **and entity** chunks) run serially under the task monitor | `WorldRegionUpdateTask.java:91-152,166-168`; `C/world/mca/MCAWorld.java:123-126` | 0.5–2 s fully serial per region; about 10–40% wall clock at high thread counts | parallelism |
| 4 | No visibility early-out: every buried block runs the full model path, and air above the terrain is walked block by block | `C/map/hires/block/BlockRenderPass.java:68-95`, `ResourceModelRenderer.java:181-213` | Probably the **largest CPU sink** once #1–3 are fixed. Est. 3–10× of hires CPU (Nether: issue #56) | redundant work |
| 5 | Caffeine on the per-block hot path: `expireAfterAccess` + `recordStats` + deep-equals keys | `C/util/Caches.java:34-53`, `ResourcePack.java:105-106,344-357`, `BlockStateModelRenderer.java:50,95`, `ChunkGrid.java:65-86` | About 20% of CPU (measured) | lookup overhead |
| 6 | The `BlockNeighborhood` ring is wiped by each full-height column, so neighbours are re-resolved up to 9× | `C/world/block/BlockNeighborhood.java:47-84`, `Block.java:52-77` | 9% `getBlock` + 7.6% `getChunk` (measured) | redundant work |
| 7 | The `variants::add` capture allocates once per non-air block | `BlockStateModelRenderer.java:88` | 24% of allocation (measured) | alloc |
| 8 | PRBM is written one byte at a time through a locked `BufferedOutputStream`, then gzip-6, on the render thread | `C/map/hires/PRBMWriter.java:286-322`, `C/storage/compression/BufferedCompression.java:44`, `Compression.java:46` | 10% (measured) + about 40–60 ms of gzip per tile (≈1.2 MB raw, see 09 §7) | I/O |
| 9 | Tiles on region boundaries are rendered twice (17×17 tiles per region, not 16×16) | `WorldRegionUpdateTask.java:100-102,310-335` + `C/util/Grid.java:145-166` | **+12.9% hires renders** on a full render | redundant work |
| 10 | Each lowres column write takes a shared `ConcurrentHashMap` bin lock and an `RRWL` read-lock CAS on the same tile | `LowresLayer.java:199-207`, `LowresTile.java:64-76` | about 1–3%, and it scales badly with thread count | contention |
| 11 | The entity pass loads and parses entity chunks for every tile, although core ships no entity models | `C/map/hires/entity/EntityRenderPass.java:55-66`, `MCAWorld.java:141-164` | Worlds with many entities: one extra decompress+parse per chunk | redundant work |
| 12 | Chunk cache churn: `init()` invalidates every chunk of the region (including border chunks just decoded by the neighbouring region), and there is a 1-min `expireAfterAccess` | `WorldRegionUpdateTask.java:114`, `ChunkGrid.java:65-70` | about 3–6% extra chunk decodes, and more with several maps per world | redundant work |
| 13 | `ArrayTileModel.sort` permutes by chain-following without visited marks | `C/map/hires/ArrayTileModel.java:542-553` | Worst case O(n²). Usually fine; unverified | algorithmic |

---

## 1. One save stalls the whole pool (root cause of 17% CPU)

The lock order, outermost first:

```
RenderManager.doWork      synchronized(renderTasks) { task.hasMoreWork() }          RenderManager.java:306-318
 └ CombinedRenderTask     synchronized hasMoreWork();  doWork: synchronized(this){ region.hasMoreWork() }  :52-67
    └ WorldRegionUpdateTask synchronized hasMoreWork();  doWork: synchronized(this){ init() } / { atWork--; complete() }  :160-191
       └ complete() (synchronized) → map.getMapChunkState().set(..)  [synchronized on CellStorage]  :246
                                   → map.saveDebounced()            [synchronized on BmMap]       :263
          └ BmMap.save() (synchronized, runs on SCHEDULER or the CLI 2-min timer, BlueMapCLI.java:180-188)
               lowres PNGs + mapTileState/mapChunkState/mapRegionState gzip-NBT saves   BmMap.java:174-186
```

How the stall happens:
1. The thread that finishes a region's last tile calls `complete()` **while holding the region monitor**.
2. `complete()` blocks on `MapChunkState`/`BmMap` for as long as a save is running (avg 15 s, max 40 s).
3. Any thread that now enters `CombinedRenderTask.doWork` takes the combined monitor and calls `region.hasMoreWork()`. That call blocks, and the thread keeps holding the combined monitor.
4. `RenderManager.doWork` calls `task.hasMoreWork()` **while holding `renderTasks`**, so that call blocks too.
5. Every remaining worker then queues on `renderTasks`. This is the 2–2.5 s average monitor wait.

Separately, `processTile` ends with `map.getMapTileState().set(..)` (`WorldRegionUpdateTask.java:230`), which is `synchronized` on the same object as `CellStorage.save()`. While the map save gzips tile-state cells, every thread that finishes a tile blocks there.

Saves come often. One is scheduled 15 s after each region completes (`BmMap.java:159-172`), and the CLI adds one every 2 min. A save lasts about 15 s, so a region completion very often lands inside a save. The pool is idle for most of the render.

**Fix (Java).**
- Never call out while holding a task monitor.
- Make `hasMoreWork` a volatile read.
- Make `saveDebounced` an `AtomicBoolean` CAS.
- Move `complete()`'s state writes outside the monitor.
- Snapshot dirty state and write it on the I/O thread without holding `BmMap`.

**Rust.**
- One lock-free work source: a rayon or crossbeam queue of `(map, tile)` jobs.
- Render-state updates go through a channel to a single persistence thread that owns the state files.
- The render path takes no lock that the persistence path holds.

## 2. The save itself: lowres write amplification and per-pixel `int[]`

**Allocation (28% of all allocation).** `BufferedImage.setRGB`/`getRGB` go through `ColorModel.getDataElements(rgb, null)` and `Raster.getDataElements(x, y, null)`, and each of those calls **allocates a fresh `int[1]`**.
- `LowresTile.set` makes 2 such calls per column, plus seam duplicates (`LowresTile.java:67-71`).
- The LOD cascade in `LowresLayer.saveTile` makes 3 `getRGB` calls per source pixel: `getColor`, `getHeight` and `getBlockLight` (`LowresLayer.java:172-178`). For one 500×500 tile that is about 750k allocations per save.

**Write amplification (4–7 encodes per PNG).**
- `save()` re-encodes **every pending tile in full**: a 501×1002 image through `ImageIO` PNG, under the tile's write lock (`LowresTile.java:93-100`).
- Pending sets keep refilling:
  - (a) Lowres tiles (500) do not align with regions (512), so each LOD1 tile is touched by 1–4 regions, and it is saved after each of them.
  - (b) The seamless-edge writes mark neighbour tiles `(cellX-1)`/`(cellZ-1)` pending as well (`LowresLayer.java:213-227`).
  - (c) Saving a LOD n tile writes its 100×100 block into LOD n+1, which marks that tile pending (`:184-192`). A LOD2 tile spans about 25 regions and a LOD3 tile about 600, so they are re-encoded on almost every save for the whole render.
- An inline `save()` also fires on a render thread once 200 tiles are pending (`:203`).

**State saves under the lock.** `CellStorage.save/saveCell` gzip-writes BlueNBT while synchronized (`CellStorage.java:84-142`), and `cell()` is synchronized on the same monitor (`:97`).

**Fix.**
- Keep lowres images as plain `u32` buffers. In Java, use `((DataBufferInt) raster.getDataBuffer()).getData()`.
- Write the seam pixels only at encode time, by copying neighbour edges when encoding.
- Build LOD n+1 once per LOD n tile, after all of its contributing regions are done.
- Encode LOD2/LOD3 only at the end of the render, or when a time budget expires, never per save cycle.
- Encode on a dedicated thread from a snapshot (copy the `u32` buffer, then release).

**Rust.** Use `Vec<u32>` per tile and a `png` crate encoder with fast filtering. Track dirty state per tile and per LOD, and flush LOD n+1 only once its 5×5 children are clean.

## 3. Serial region `init()` and preload

The first `doWork` on a region runs `init()` inside `synchronized(this)` (`WorldRegionUpdateTask.java:160-169`). `init()` does the following:
- reads the region header;
- invalidates all of the region's chunks (`:114`);
- makes a tile-state pass;
- if at least 75% of tiles need rendering, calls `preloadRegionChunks`, which is **up to 1024 chunk decompress+parses on one thread**.

`MCAWorld.preloadRegionChunks` also preloads the **entity** region (`MCAWorld.java:123-126`).

Because of the chain in #1, every other worker waits. At about 0.5–1 ms per chunk, that is 0.5–2 s of fully serial time per region. With 22 threads, a region's parallel render phase is only a few seconds, so Amdahl's law costs tens of percent.

**Fix.** Make preload parallel: one job per chunk or per 32-chunk row. Better, make chunk decode the first stage of the tile job graph, with each tile depending on its ≤4 chunks, so decode overlaps rendering.

**Rust.** Use a per-region arena. Decode chunks with rayon `par_iter` over the region header's present chunks, then render tiles. Shared border chunks come from the neighbouring region's arena or a small LRU.

## 4. No visibility early-out (largest CPU sink after the stalls)

`renderTopOnly` is false for the default perspective/free-flight maps. In that case `BlockRenderPass` walks **every y** from the top of the highest section (usually the light-padding section, about y=335) down to `sectionMin*16` (`BlockRenderPass.java:69-73`). Each block costs:
- `block.set`;
- a mask test (`isInsideRenderBounds`);
- a palette decode;
- a light decode for `topBlockLight` (`:82`, which also runs for air).

**Buried solid blocks** (stone and netherrack, about 60–70% of a column) go through the full path:
1. Resource-state Caffeine get.
2. Variant matching plus a lambda allocation.
3. Renderer Caffeine get.
4. Per element, six `createElementFace` calls. Each one:
   - fetches the faced neighbour **for light first** (`ResourceModelRenderer.java:188-193`);
   - runs the cave test (`:196-199`);
   - fetches the cull neighbour a **second** time (`:208-213`, another matrix rotation + `Math.round`), then does a properties lookup.
5. Element and variant transform sweeps over zero faces.

That is about 12 neighbour resolutions, 6 light decodes and 6+ property lookups per block, **to emit nothing**. A 32×32 tile has about 1024×250 ≈ 250k such blocks.

The Nether shows this most clearly. Every column is solid from the y=127 roof down, which matches the 7–8× slowdown in issue #56.

**Fix.**
- **Fully enclosed skip:** if a block is culling/opaque and all 6 face neighbours are `culling`, skip it before resolving variants. One bit test per neighbour on a dense occupancy mask.
- **Cave-dark skip:** for `isRemoveIfCave` blocks, if all 6 neighbours and the block itself have sky light 0 (or max(sky, block) = 0 when `caveDetectionUsesBlockLight`), skip. This is the exact face-level predicate hoisted to block level, so output is unchanged.
- **Clamp the top** to `max(WORLD_SURFACE over the 3×3 columns) + 1`. Above that height every block is air by definition, and the heightmap is already parsed but unused (01 §2).
- Reorder `createElementFace` so the cull test runs **before** the light fetch.

Rust: build these masks per tile from the dense padded volume (03 §8). Estimated hires-CPU gain: 3–10× (overworld lower, Nether higher). This is an estimate, not a measurement.

## 5. Caffeine on the hot path (~20% of CPU)

`Caches.build` (`Caches.java:41-53`) gives every cache `maximumSize(10000)`, `expireAfterAccess(1 min)` and `recordStats()`. On a hit, each `get` pays for:
- a ticker read (`nanoTime`, from `hasExpired`);
- a read-buffer offer and access-time write (`afterRead`);
- a `LongAdder` stats increment;
- `hashCode`, plus a deep `equals`.

The deep `equals` happens because palette `BlockState`s are separate instances: id + `Property[]` compare (`world/BlockState`, see 01 §3).

These caches are hit per block or per neighbour:
- `ResourcePack.blockStateCache` and `blockPropertiesCache` (`ResourcePack.java:105-106,344-357`): once per non-air block, and once per neighbour-slot reset.
- `BlockStateModelRenderer.blockRenderers` (`:50,95`): a **Caffeine cache over a 3-entry registry**, hit once per variant per block.
- `ChunkGrid.chunkCache` (`:65-86`): once per neighbour-slot x/z change, plus a `Vector2iCache` lookup.

Vanilla has about 27k block states, more than the 10k cap. Varied areas (builds, or modded content) can churn the property cache, and each miss rebuilds `BlockProperties`, including model `forEach`.

**Fix.**
- Intern states to `u32` at chunk load.
- Store properties, resolved variants and renderer in dense arrays indexed by state id.
- For chunk lookup, index a region arena directly.

In Java, even replacing these with `ConcurrentHashMap` (no expiry, no stats) and an `EnumMap`/array for renderers would recover most of the 20%.

## 6. The neighbourhood ring is defeated across columns

`BlockNeighborhood` is an 8×8×8 ring indexed by `coord & 7` (`BlockNeighborhood.java:80-84`). The traversal is x → z → y, with y descending over the full height (#4). By the time column (x, z+1) starts at the top, every ring slot for (x±1, ·, z) holds blocks near the **bottom** of the previous column.

So each neighbour position is re-resolved once per centre column that needs it, up to 9× for AO and biome blending, and 5× for culling. Each re-resolution redoes the palette decode, light decode and property Caffeine lookup. Each slot's x/z change also resets `Block.chunk` (`Block.java:53-57`), which triggers a `ChunkGrid.getChunk` Caffeine lookup.

The biome blend (5×3×5 = 75 slots) churns the ring further. This matches `getBlock` 9% + `getChunk` 7.6%.

**Fix (Rust).** Use a dense padded per-tile volume of `u16` state ids, `u8` packed light and a biome index, decoded once. A neighbour is then `base + offset`. In Java, the minimum fix is a per-column-slab cache keyed by (x, z) over the full height.

## 7. Lambda allocation per block (24% of allocation)

`stateResource.forEach(blockState, x, y, z, variants::add)` (`BlockStateModelRenderer.java:88`) creates a new capturing `Consumer` for every non-air block, and twice for waterlogged blocks. `forEach` dispatches through `Variants`/`Multipart` polymorphically, so escape analysis does not remove the allocation.

**Fix:** hoist it to a final field (`private final Consumer<Variant> addVariant = variants::add;`). The same pattern applies to `modelResource.getTextures()::get` per face (`ResourceModelRenderer.java:236`).

In Rust, resolve the variant list once per state id at load. Only the weighted pick stays per position.

## 8. PRBM serialisation and compression on the render thread

`PRBMWriter` writes every byte through `CountingOutputStream.write(int)` → `BufferedOutputStream.write(int)` (`BufferedCompression.java:44`). On JDK 21+ that path takes an internal lock and calls `growIfNeeded` on every byte. At about 1.2 MB raw per tile (09 §7), that is about 1.2M locked calls, which is the measured 10%.

Then `GZIPOutputStream` at the default level 6 (`Compression.java:46`) costs about 40–60 ms per tile, on the render thread.

**Fix.**
- Assemble PRBM into one `byte[]`/`Vec<u8>` with bulk `ByteBuffer.order(LE).asFloatBuffer().put(...)` (Rust: `bytemuck`).
- Compress once with zlib-ng/libdeflate at level 1–4, or store zstd (see 09).
- Hand the compress and write step to an I/O pool, so render threads only build geometry.

## 9. Boundary tiles are rendered twice

The hires grid has offset 2, so region r covers tiles `floorDiv(512r-2, 32) .. floorDiv(512r+509, 32)`, which is 17 tiles per axis (`Grid.getCellMin/Max`, `WorldRegionUpdateTask.java:100-102`). Each boundary tile row and column belongs to 2 regions, and corner tiles to 4.

`checkChunksHaveChanges` (`:310-335`) only compares chunks inside the current region. On a full or forced render, both regions therefore re-render the shared tiles. The cost is 289/256 = **+12.9% hires work**.

**Fix:**
- Give each tile a single owner region for full renders. The tile is still rendered when any of its covered chunks changed in any region: compare stored chunk hashes for **all** covered chunks, using each chunk's own region header.
- Or schedule a deduplicated tile set across regions.

## 10. Per-column lowres contention

For every column, `LowresLayer.accessTile` does a `Vector2iCache` get, a Caffeine `tileCache.get`, and `pendingChanges.put(tilePos, tile)` (`LowresLayer.java:199-207`).

The put is a `ConcurrentHashMap` replace that **synchronizes on the bin head**. All threads hit the same 1–4 keys, because a region maps to about one lowres tile. `LowresTile.set` also takes `RRWL.readLock()`, a CAS on one shared counter (`LowresTile.java:65`). Together that is 1024 contended lock operations per hires tile, plus cache-line ping-pong.

**Fix:**
- Accumulate a tile's 32×32 column results locally and publish once per hires tile.
- In Rust, send `(tile, [Column; 1024])` to a single lowres aggregator thread, or write disjoint rows with no lock.

## 11. Entity pass is mostly wasted

`EntityRenderPass` runs for every tile (the pass registry holds both passes, `RenderPassType.java:38-44`). It calls `world.iterateEntities`, which loads and parses `entities/` chunks through a second `ChunkGrid` (`MCAWorld.java:141-164`), and the preload pulls in the whole entity region (`:125`).

Core ships only `entitystates/missing.json`, so unknown entities are skipped after parsing (03 §7). For worlds with item frames, armour stands, villagers and mob farms, that is one extra decompress+parse per chunk for no output.

**Fix:** skip the pass, and do not preload entity regions, when the resource pack defines no entitystates beyond `missing`. Parse lazily per entity id in Rust.

## 12. Chunk cache churn

`init()` invalidates every chunk listed in the region header (`WorldRegionUpdateTask.java:114`), whether or not its timestamp changed. That includes the border chunks that the previous region's boundary tiles decoded moments earlier. Two maps over the same world (shared `World`) each invalidate and re-decode.

`expireAfterAccess(1 min)` (`ChunkGrid.java:68-69`) can also drop the border chunks of slow regions before the neighbouring region needs them.

**Fix:** invalidate only chunks whose header timestamp differs from the cached chunk's timestamp. In Rust, key the region arena by `(region, header fingerprint)`.

## 13. Sort permutation

After the index merge sort, `sort()` applies the permutation in place by following `materialIndexSort[s]` until `s >= i` (`ArrayTileModel.java:544-553`). There are no visited marks. This is O(n log n) on average for random cycles but O(n²) in the worst case, and the code guards against an endless loop with an `IllegalStateException`.

**Fix:** use an out-of-place gather into the pooled second buffer, or a counting sort by material while emitting faces (03 §8).

---

## Implications for the Rust port (new, beyond 03 §8)

1. **Scheduler:** a lock-free tile job graph (chunk decode → tile render → compress/write → lowres aggregate). Use one persistence thread for render state and lowres. Never block render workers on I/O.
2. **Visibility pre-pass per tile:**
   - occupancy and culling bitmask;
   - skip fully enclosed blocks and blocks that are cave-dark on all sides;
   - start each column at `max(WORLD_SURFACE over 3×3) + 1`.
3. **Dense ids everywhere:** state id → (properties, resolved variants, renderer) arrays. No hash lookups in the block loop.
4. **Tile dedup across regions** for full renders, and change detection over all covered chunks.
5. **Lowres:**
   - `Vec<u32>`, publishing once per hires tile;
   - seams applied at encode time;
   - each LOD written once, after its children settle.
6. **Skip the entity pass** unless entity models exist.
7. **Benchmark gates:** CPU utilisation across threads (Java reaches about 17% on 22 threads), tiles/s, and PNG writes per tile (target 1).
