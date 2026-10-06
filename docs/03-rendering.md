# 03 — Map rendering pipeline (hires, lowres, render state, masks)

Path prefixes: `M/` = `core/src/main/java/de/bluecolored/bluemap/core/map/`, `C/` = `core/src/main/java/de/bluecolored/bluemap/core/`, `CM/` = `common/src/main/java/de/bluecolored/bluemap/common/`, `W/` = `common/webapp/src/js/map/`.

## 1. Tile grid

- `Grid(size, offset)`: `cell = floorDiv(pos - offset, size)`, `min = cell*size + offset`, `max = (cell+1)*size + offset - 1` (`C/util/Grid.java:85-136`).
- **Hires**: `Grid(hiresTileSize=32, offset=2)` (`M/BmMap.java:119`, default `CM/config/MapConfig.java:100`). Tile (0,0) covers blocks x,z ∈ [2,33]. A tile can straddle two regions/chunks.
- **Lowres**: `Grid(lowresTileSize=500)`, no offset (`M/BmMap.java:124`). `lodCount=3`, `lodFactor=5` (`MapConfig.java:101-103`). LOD 1 = 1 px per block; LOD n+1 = 5×5 average of LOD n. So LOD1 = 500 blocks, LOD2 = 2500, LOD3 = 12500 blocks per tile edge, all 500 px.
- `settings.json` exposes `hires.tileSize`, `hires.scale=[1,1]`, `hires.translate=offset`, `lowres.{tileSize,lodFactor,lodCount}` (`M/MapSettingsSerializer.java:50-66`).
- Region update: tiles for a region = `regionGrid.getCellMin/Max(regionPos, tileGrid)` (`CM/rendermanager/WorldRegionUpdateTask.java:100-102`). Tiles are processed row-major (x inner) one tile per `doWork()` call, so several render threads share a region task (`:154-192`).

## 2. Hires render

### Driver
- `HiresModelManager.render` (`M/hires/HiresModelManager.java:71-100`): `modelMin = (tileMinX, MIN_INT, tileMinZ)`, `modelMax = (tileMaxX, MAX_INT, tileMaxZ)`, `anchor = (tileMinX, 0, tileMinZ)`. Claims a pooled `ArrayTileModel`, runs each `RenderPass` (BLOCKS, ENTITIES — `M/hires/RenderPassType.java:38-44`; registry is a `ConcurrentHashMap`, so pass order is unspecified), catches `MaxCapacityReachedException`, `model.sort()`, writes PRBM, recycles. If `enableHires=false` it renders into `VoidTileModel` only to feed lowres.
- Render passes are `ThreadLocal` per render thread (`HiresModelManager.java:62`).

### Per-block iteration (`M/hires/block/BlockRenderPass.java:53-104`)
- `for x in minX..=maxX { for z in minZ..=maxZ { for y in maxY down to minY } }` — x outer, z middle, y descending.
- Column skipped entirely if `!isInsideRenderBoundaries(x,z)` (mask XZ test, `M/hires/RenderSettings.java:74`).
- `minY/maxY` = chunk section bounds (`sectionMin*16`, `sectionMax*16+15`, e.g. `C/world/mca/chunk/Chunk_1_18.java:175-182`).
- Per block: `block.set(x,y,z)`; skip if `!isInsideRenderBounds()` (3D mask test); render model; `model.translate(x-anchorX, y, z-anchorZ)` → **positions are tile-local in X/Z, absolute in Y**.
- Lowres column accumulation: if `blockColor.a > 0` → `maxHeight = max(y)`, `columnColor.underlay(blockColor.premultiplied())` (front-to-back compositing). `topBlockLight = max(topBlockLight, blockLight * (1 - columnColor.a))` computed *before* the underlay.
- `renderTopOnly` (= `!enableHires || (!perspective && !freeFlight)`, `M/MapSettings.java:71-73`): break after first block with `a > 0.999 && culling`; also drops faces whose rotated normal `y < 0.01` (`M/hires/block/ResourceModelRenderer.java:207`).
- After column: `tileMetaConsumer.set(x, z, columnColor, maxHeight (0 if none), (int) topBlockLight)` → lowres.

### Block → variants → renderer (`M/hires/block/BlockStateModelRenderer.java:58-109`)
- Air short-circuit. `resourcePack.getBlockState(state).forEach(state, x,y,z, variants::add)` (multipart + weighted random by position — resolved per block!).
- Each variant dispatched to renderer type `bluemap:default` (ResourceModelRenderer), `bluemap:liquid`, `bluemap:missing` (`M/hires/block/BlockRendererType.java:37-45`).
- Block map-color = sum of premultiplied variant colors, flattened, alpha = max variant alpha.
- **Waterlogging**: if `state.isWaterlogged() || properties.alwaysWaterlogged`, render `BlockState.WATER` as an extra model in the same block; color = `water.overlay(block.premultiplied())` (`:70-74`).

### ResourceModelRenderer (`M/hires/block/ResourceModelRenderer.java`)
- Per element: 8 corners in 0..16 space; faces in order DOWN, UP, NORTH, SOUTH, WEST, EAST with fixed corner tuples (`:155-170`). Each face = **2 triangles**: (c0,c1,c2), (c0,c2,c3) (`:224-233`).
- After faces: transform by `element.rotation.matrix × scale(1/16)` (`:174-177`). After all elements: variant transform `translate(-.5) · rotateYXZ(-x,-y,-z) · translate(.5)` (`C/resources/pack/resourcepack/blockstate/Variant.java:78-84`), applied only if any rotation ≠ 0.
- **Light** per face: `sky = max(self.sky, facedNeighbor.sky)`, `block = max(self.block, facedNeighbor.block)`, neighbour = face direction rotated by variant (`:188-193`). Blocklight attribute = `max(blockLight, element.lightEmission)` (`:298`).
- **Cave removal**: face dropped if `block.isRemoveIfCave() && (usesBlockLight ? max(bl,sl) : sl) == 0` (`:196-199`). `isRemoveIfCave = y < removeCavesBelowY(55) && (!hasOceanFloorY || y < oceanFloorY + caveDetectionOceanFloor(10000))` (`C/world/block/ExtendedBlock.java:159-171`).
- **Culling**: if face has `cullface`, neighbour (rotation-relative) with `culling` property → drop; `cullingIdentical && sameState` → drop (`:208-213`).
- **UV**: `uv/16`, raw corners `[(u0,v1),(u1,v1),(u1,v0),(u0,v0)]`, rotated by `floorDiv(rotation,90)%4` steps (`:249-258`). **UV lock**: if `uvlock && transformed`, rotate UVs around (0.5,0.5) by angle between rotated face-up and projected world-up (`:261-269`, `:346-377`).
- **Tint**: `tintindex >= 0` → `blockColorCalculator.getBlockColor(block)` lazily once per variant (`:285-295`), else white. Calculators: foliage/dry_foliage/grass = colormap + biome overlay (+grass modifier), water = biome water color, all **blended 5×3×5 (h=2, v=1) = 75 samples** (`M/hires/block/color/BlockColorCalculatorType.java:38-60`, `BlendedBlockColorCalculator.java:47-90`); redstone = power-based.
- **AO** (only if model `ambientocclusion`): per corner, check up to 4 neighbours (edge x/y, x/z, y/z, corner) that lie on the face's side; `ao = clamp(1 - min(occluding,3)*0.25)` (`:408-452`). Corner detection uses exact `==0`/`==16` coords. Triangle AO: (ao0,ao1,ao2),(ao0,ao2,ao3).
- **Map color**: for faces with rotated normal `y > 0.01`: `texture.colorPremultiplied × tint × light`, `light = (1-ambient)*max(sl,bl)/15 + ambient`; summed, alpha=max (`:319-340`).
- **Random offset** (`randomOffset` property): `dx = (h(x,z,123984)-0.5)*0.75`, `dz` seed 345542; `h = ((x*73428767L ^ z*4382893L ^ seed*457) * (hash+456149) & 0xFFFFFF) / 2^24` (`:131-135`, `:454-457`). No Y offset.

### LiquidModelRenderer (`M/hires/block/LiquidModelRenderer.java`)
- Waterlogged → treated as WATER (`:112-113`). Same cave filter on own light (`:124-127`).
- Top corner heights (0..16 units): if `level < 8` and not (level 0 with same liquid above): per corner `getLiquidCornerHeight` (`:130-140`, `:190-228`): any same-liquid in the 2×2 above → 16; any same-liquid level-0 in the 2×2 → 14; else average of `14 - level*1.9` (level≥8 → 16) over same-liquid + non-blocking (air) neighbours; fallback 3.
- Same-liquid for water includes waterlogged/alwaysWaterlogged blocks (`:234-241`).
- Faces culled if neighbour is same liquid, or (non-UP and neighbour `culling`) (`:259`). No AO (all 1). Light: UP uses own light, sides use the neighbour's (`:340-346`).
- UVs: UP still unless flowing; flow angle from height gradient → UV `translate(-.5) scale(.5) rotate(-angle) translate(.5)` with `flow` texture; sides always flow texture scaled 0.5 (`:287-312`, `:358-374`). DOWN = still.
- Map color only if UP face rendered; `light = (ambient + l/15)/(ambient + 1)` (differs from solid formula) (`:171-187`).

### Missing models
`MissingModelRenderer` delegates to a fallback renderer type or DEFAULT with BlueMap's "missing" model (`M/hires/block/MissingModelRenderer.java:41-59`).

### Neighbour access & render edges
- `BlockNeighborhood`: 8×8×8 ring cache of `ExtendedBlock`s keyed by `(x&7,y&7,z&7)` (`C/world/block/BlockNeighborhood.java:33-84`).
- `ExtendedBlock.getBlockState()` returns AIR outside render mask when `renderEdges` (default true); light outside → sky = `edgeLightStrength` (15) if dimension has skylight (`C/world/block/ExtendedBlock.java:101-111`). This makes faces at the mask boundary render ("cut-away" edges). Mask is cached per 16×16 XZ area via `submask` (`:173-203`).

### Vertex buffer: `ArrayTileModel` (`M/hires/ArrayTileModel.java`)
- **SoA, face(triangle)-major** primitive arrays (`:60-75`): `position f32[9/face]`, `uv f32[6/face]`, `ao f32[3/face]`, `color f32[3/face]` (per face, not per vertex), `sunlight i8[1]`, `blocklight i8[1]`, `materialIndex i32[1]`.
- Grows ×1.5 + count, `MAX_CAPACITY = 1_000_000` triangles → `MaxCapacityReachedException`, tile saved truncated (`:485-507`).
- Pooled via `InstancePool` (starts at 100, dropped after 1 min of <66% use) (`:43-57`).
- `TileModelView` = (start,size) window used to apply transforms to just the faces a block/element added (`M/hires/TileModelView.java`).
- Transforms operate in place over a face range: quaternion rotate (double math, `:349-375`), scale, translate, 3×3/4×4 matrix (`:448-471`). `invertOrientation` swaps v0/v2.
- `sort()`: stable merge-sort of face indices by `materialIndex` then in-place permutation swaps (`:530-554`, `C/util/MergeSort.java`). Material index = **texture id from `TextureGallery`**, which assigns ids with opaque textures first, then translucent, each alphabetical, `missing` = 0 (`M/TextureGallery.java:76-88`) — so sorting by material implicitly puts opaque before transparent. Ids are persisted in `textures.json` and only appended.

## 3. Hires output: PRBM (`M/hires/PRBMWriter.java`)

Adapted PRWM (Kevin Chapelier) — reader `W/hires/PRBMLoader.js:112-245`. **Non-indexed, little-endian, uncompressed bytes; the whole stream is then wrapped by storage compression (default GZIP).** Padding is computed from the count of *uncompressed* bytes written.

```
off  size  value
0    1     0x01                 format version
1    1     0x07                 flags: bit7 indexed=0, bit6 idxType=0, bit5 bigEndian=0, bits0-4 attrCount=7
2    3     u24 LE  N = faces*3  vertex count (max 0xFFFFFF)
5    3     u24 LE  0            index count
then 7 attributes, each:
     name ASCII + 0x00
     1 byte attr flags: bit7 type(0=float,1=int) | bit6 normalized | bits4-5 cardinality-1 | bits0-3 encoding
     zero padding to 4-byte boundary
     data (N * cardinality elements, LE) — NO padding after data
after attributes: pad to 4, then groups: repeat {i32 materialIndex, i32 start(vertex), i32 count(vertices)}, terminator i32 -1
```
Encodings: 1=f32, 3=i8, 4=i16, 6=i32, 7=u8, 8=u16, 10=u32.

| # | name | flag byte | per-vertex data | source |
|---|------|-----------|-----------------|--------|
| 1 | `position` | 0x21 (float, 3D, f32) | 3×f32 tile-local | `:88-105` |
| 2 | `normal` | 0x63 (normalized, 3D, i8) | `(byte)(n*128 - 0.5)` (double, truncating), same for all 3 verts; n = normalize((p2-p1)×(p3-p1)) | `:107-137`, `:310-313`, `:325-344` |
| 3 | `color` | 0x67 (normalized, 3D, u8) | `(int)(c*255) & 0xFF`, face color repeated ×3 | `:139-160`, `:315-318` |
| 4 | `uv` | 0x11 (2D, f32) | 2×f32 | `:162-179` |
| 5 | `ao` | 0x47 (normalized, scalar, u8) | `(int)(ao*255)` | `:181-198` |
| 6 | `blocklight` | 0x03 (scalar, i8, not normalized) | 0..15 raw, face value ×3 | `:200-219` |
| 7 | `sunlight` | 0x03 | 0..15 raw, face value ×3 | `:221-240` |

- Groups: consecutive runs of equal `materialIndex` after sort; `start`/`count` in vertices (= faces*3). Empty model → just padding + `-1` (`:242-277`).
- Degenerate triangle → NaN normal → `(byte)NaN = 0`; Rust `f64 as i32` also yields 0 — matches.
- Rust gotcha: replicate `(byte)(double)` = saturate to i32 then truncate to i8; `(int)(f*255) & 0xFF` = `(f*255.0) as i32 as u8`.
- File path: `<map>/tiles/0/` + digit-split path + `.prbm` + compression suffix (`.gz` default; `.deflate`, `.zst`, `.lz4`, none) (`C/storage/file/FileMapStorage.java:65-70`, `C/storage/compression/Compression.java:45-49`). Digit split: `"x-12z5"` → `x-1/2/z5.prbm.gz` — a new dir after every digit, last segment is the file (`C/storage/file/FileGridStorage.java:111-133`).
- Bit-exact floats are not needed for the client, but if golden-file tests are wanted: rotations use flow-math `TrigMath.sin/cos` (table-based approximation) in `C/util/math/MatrixM4f.java:89-173` and `ArrayTileModel.java:236-283`; quaternion math in double.

## 4. Lowres

- Fed per column by the hires block pass via `TileMetaConsumer.set(x,z,color,height,blockLight)` (`M/lowres/LowresTileManager.java:91-97`) — LOD1 is exactly 1 px per block column, derived from the hires traversal (no separate pass).
- `LowresTile` = ARGB image **(size+1) × 2(size+1)** = 501×1002 (`M/lowres/LowresTile.java:46-49`). +1 row/col duplicates the neighbour tile's first pixel for seamless edges (`M/lowres/LowresLayer.java:209-228`).
  - Top half (y < 501): straight-alpha color `ARGB = (a,r,g,b)*255` truncated (`C/util/math/Color.java:64-70`).
  - Bottom half (y = 501+z): `A=0xFF, R=blockLight, G=height>>8 & 0xFF, B=height & 0xFF` (16-bit two's-complement height) (`LowresTile.java:64-76`). Read-back: `h & 0xFFFF`, sign-extended if `> 0x8000` (`:82-87`). Client: `height = g*65280 + b*255`, light = `r*255` (`W/lowres/LowresVertexShader.js:40-41`, `LowresFragmentShader.js:70-71`).
- LOD downsampling on save of a tile (`LowresLayer.java:150-194`): for each 5×5 group of the 500×500 core: color = mean of premultiplied colors; height and light = integer mean (truncating). Written to next LOD tile `floorDiv(tile,5)` at pixel `floorMod(tile,5)*100 + g`. Cascade happens when the next layer's pending tile is saved.
- Pending changes: map of dirty tiles; flush when ≥200 pending or on `save()`; tiles held in weak+soft Caffeine caches (`:51-93`, `:199-207`). `set()` takes the *read* lock, PNG save takes the *write* lock (concurrent disjoint pixel writes, exclusive encode) (`LowresTile.java:64-100`).
- Unrender sets transparent color, height 0, light 0 for the tile's columns (`HiresModelManager.java:106-117`).
- Files: `<map>/tiles/{1..lodCount}/<digit-split>.png`, never compressed (`FileMapStorage.java:72-77`). PNG encoder is ImageIO; byte-equality not required, only RGBA8 semantics.

## 5. Render state / change detection

- Stored as gzip'd BlueNBT under `<map>/rstate/` (`FileMapStorage.java:79-98`), cell index `(z & mask) << SHIFT | (x & mask)`:
  - `*.tiles.dat` — `MapTileState`, 32×32 hires tiles per file (SHIFT 5): `last-render-times int[1024]` (unix secs), `tile-states` paletted `TileState[]` keyed `bluemap:<name>` (`M/renderstate/MapTileState.java:33`, `TileInfoRegion.java:48-52`, `CellStorage.java:49-53`).
  - `*.chunks.dat` — `MapChunkState`, 128×128 chunks (SHIFT 7): `chunk-hashes int[16384]` (`ChunkInfoRegion.java:40`).
  - `regions/*.regions.dat` — `MapRegionState`, 64×64 regions (SHIFT 6): `last-update-times int[4096]` (`RegionInfoRegion.java:40`).
- "Chunk hash" is actually the **MCA region header timestamp** per chunk (`WorldRegionUpdateTask.java:108-115`). A tile is "changed" if any of its chunks inside the current region has a different stored timestamp (`:310-335`); hashes committed after the whole region completes (`:239-250`).
- `TileState` state machine (`M/renderstate/TileState.java:37-90`): unknown, rendered, rendered-edge, out-of-bounds, not-generated, missing-light, low-inhabited-time, chunk-error, render-error. Input = (changed||force, bounds ∈ INSIDE/EDGE/OUTSIDE from mask `test(...).getOr(true/false)`) → action NONE/RENDER/DELETE + next state. Pre-render checks: errored chunk, not generated / no light (unless `ignoreMissingLightData`), `minInhabitedTime` (+radius) (`WorldRegionUpdateTask.java:345-388`) — failures unrender and record that state.
- Other map files: `settings.json` (`MapSettingsSerializer`), `textures.json(.gz)` (texture array: `key,color,halfTransparent,texture(data-url),animation`; `M/TextureGallery.java:90-106`), `live/markers.json`, `live/players.json` (`{}` at start) (`M/BmMap.java:224-252`, `FileMapStorage.java:144-161`). Saves are debounced 15 s (`BmMap.java:62`, `:159-172`).

## 6. Masks, bounds, edges, caves

- `Mask` API: `test(x,y,z)`, `test(box) -> Tristate`, `isEdge(xz box)`, `submask(box)` for per-area optimisation (`M/mask/Mask.java`).
- `CombinedMask`: ordered layers `(mask, value)`; last matching layer wins; first `value=false` layer auto-prepends `ALL=true`; empty = everything (`M/mask/CombinedMask.java:38-64`). Types: Box (min/max xyz), Ellipse, Polygon (even-odd ray test + edge/rect intersection, minY/maxY) (`PolygonMask.java:39-106`), Blur (jitters test coords by hash noise ±size) (`BlurMask.java:37-64`).
- Min/max Y bounds live inside masks (no separate min/max Y config anymore — legacy keys rejected, `MapConfig.java:130-142`).
- Render edges + edge light: see §2 neighbour access. Caves: `removeCavesBelowY=55`, `caveDetectionOceanFloor=10000` (relative to OCEAN_FLOOR heightmap), `caveDetectionUsesBlockLight=false` (`MapConfig.java:74-76`).

## 7. Entities

- `EntityRenderPass` iterates entities whose floored pos lies in the tile XZ range (`C/world/mca/MCAWorld.java:141-164`), sets the neighbourhood to the entity's block, renders, translates by exact double pos − anchor (`M/hires/entity/EntityRenderPass.java:55-67`).
- Model from `entitystates/<id>.json` → parts `{renderer, model, transform}`; applies `rotateYXZ(0, 180 - yaw, 0)` (`M/hires/entity/EntityModelRenderer.java:48-68`). Renderer = same element/face/UV code as blocks but **no culling, no AO, light from entity's own block only, no tint (NO_TINT)** (`M/hires/entity/ResourceModelRenderer.java:99-254`). Cave filter applies.
- Core ships only `bluemap:entitystates/missing.json`; unknown entity ids with no entitystate are skipped (`EntityModelRenderer.java:49-50`). So entities are effectively opt-in via resource packs/addons. Port it late.

## 8. Hot loops, perf, and Rust opportunities

Observed Java costs:
- **Variant resolution per block** (`BlockStateModelRenderer.java:88`): multipart condition matching + weighted pick for every block.
- **Model building per block**: build faces in 0..16 space, then element matrix pass, scale, variant matrix pass, random-offset pass, translate pass — 4–5 sweeps over the same vertices per block (`ResourceModelRenderer.java:126-135`, `:174`; `BlockRenderPass.java:85`).
- **Neighbour lookups**: every face does rotation-relative neighbour lookups with a matrix multiply + `Math.round` (`ResourceModelRenderer.java:392-406`); AO adds up to 4 per vertex → ~100 lookups/opaque block, each through `ExtendedBlock.set` + `getProperties()` hashmap lookup.
- **Biome blending**: 75 biome samples + colormap lookups per tinted block (grass/leaves/water — most of the surface).
- Liquids: `getLiquidCornerHeight` does up to 8 neighbour reads per corner × 4.
- PRBM writing is byte-at-a-time `OutputStream.write(int)` through counting + GZIP streams; normals computed at write time.
- Sort = index merge sort + swap permutation across 7 arrays.
- Mutable `Color` with a `premultiplied` flag that methods silently convert — error-prone; `LowresTile.set` mutates the caller's color (`.straight()`).
- Good already: SoA primitive arrays, pooled models, reused `VectorM3f`/`Color` scratch objects, thread-local render passes, no per-vertex allocation.

Rust plan:
- **Bake at resource-load time**: per `(BlockState, variant)` a list of pre-transformed quads (positions in 0..1 block space, UVs post-rotation/uvlock, cull direction already rotated, AO-corner neighbour offsets precomputed, tint flag, texture id, "is top face" for map color). Runtime per block = memcpy + translate + light/AO/tint fill. Only weighted variant pick stays runtime (keep Java's position hash for parity).
- **Chunk-local dense block/light/property arrays**: decode a 3×3 chunk neighbourhood (or tile + 1 border) into flat `u16 state-id` / `u8 light` / bitflag property arrays; neighbour = index offset. Removes the 8³ object cache.
- **Biome tint**: compute per tile a padded (W+4)×(H+4)×(Y slice) biome-color grid once, separable box blur → O(1) per block.
- **Buffers**: per-thread reusable `Vec`s (rayon `map_init` / `thread_local!`), SoA as now; bucket faces by material id while emitting (Vec per material, or counting sort on `u32` keys) instead of post-sort. Must stay stable within a material if golden tests compare bytes.
- **Writer**: assemble the whole PRBM into one `Vec<u8>` (`bytemuck` slices for f32/u8 arrays, `extend_from_slice`), then compress once (flate2/zlib-ng gzip, or zstd). Use `#[repr(C)]` + LE assert.
- **Parallelism**: rayon over tiles within a region (and across regions); lowres writes via per-tile `Mutex`/atomic rows or by returning 32×32 column results to a single lowres aggregator thread (no shared locking in the hot loop).
- **Lowres**: keep LOD images as `Vec<u32>`; encode PNG with the `png` crate at fast compression; cascade LOD averaging on flush.
- **SIMD**: transforms/normals over SoA f32 are trivially autovectorisable (`glam` `Vec3A` or plain loops); quantisation to i8/u8 likewise.
- Colour types: separate `Premul` and `Straight` newtypes to make Java's implicit conversions explicit.
