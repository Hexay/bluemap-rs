# 02 — Resource & data pack loading

Path prefix: `C/` = `core/src/main/java/de/bluecolored/bluemap/core/`. `RES/` = `C/resources/`. `RP/` = `RES/pack/resourcepack/`.
Line numbers refer to the cloned BlueMap source.

## 1. Obtaining the client jar; pack layering

**Client jar download** (`RES/MinecraftVersion.java`, `RES/VersionManifest.java`)
- Manifest: `https://piston-meta.mojang.com/mc/game/version_manifest.json` (`VersionManifest.java:48-49`). Version `url`s must start with that domain, and ids must not contain `/`, `..` or `\` (`:147`, `:201`).
- `id == null` means use `latest.release` (`MinecraftVersion.java:96`).
- **Two jars may be needed.** The resource-pack jar is `max(version, 1.13)` and the data-pack jar is `max(version, 1.19.4)`, compared by `releaseTime` (`:99-103`). Old worlds therefore pull a newer client for datapack biomes.
- Cache file: `<data>/minecraft-client-<id>.jar` (`:185-188`). The download goes to `*.unverified`, its SHA-1 is checked against `downloads.client.sha1` from the version-detail JSON, and the file is then atomically moved into place (`:145-183`). The connection and read timeouts are 10 s.
- If the manifest fetch fails, the code falls back to an existing local jar for `id` (`:113-120`). If the jar fails to parse, it is deleted so the next run downloads it again (`:134-141`).
- Pack format comes from `version.json` inside the jar, field `pack_version`. That field is either an int (old format) or `{resource_major|resource, resource_minor, data_major|data, data_minor}`, and both default to 4 (`:229-276`). A jar without `version.json` is treated as 1.13–1.14.4.

**Pack roots order** (`common/.../BlueMapService.java:294-390`). Highest priority comes first:
1. `packsFolder` entries, sorted **reverse lexicographic** (`:359-367`).
2. Extra roots (world datapacks, datapack only, `:323`).
3. `modsFolder/*.jar`, only if `scanForModResources` is set (`:373-382`).
4. `<data>/defaultBlockstates.zip`, generated from config as `data/<ns>/defaultBlockstates.json` (`:435-473`).
5. `<data>/resourceExtensions.zip`, BlueMap's bundled pack, rewritten on every start (`:415-433`).
6. The vanilla client jar, added last (`:302`, `:324`).
- **First loaded wins.** `ResourcePool.load` skips keys that are already present (`RES/pack/ResourcePool.java:73-84`). Atlases are the exception: they **merge** (`:86-99`, `ResourcePack.java:171-178`).

**Per-root recursion** (`RES/pack/Pack.java:61-156`)
- If the root is not a directory, it is opened as a zip FileSystem and each fs root is recursed into (`:66-76`). Zip, jar and folder are all handled the same way.
- `fabric.mod.json` → `jars[].file` nested jars are loaded **before** the outer pack (`:79-98`).
- `pack.mcmeta`: `features.enabled` is checked against `enabledFeatures`. That check is effectively unused (always null for ResourcePack/DataPack) (`:115`).
- Nested datapacks at `data/*/datapacks/*` are loaded before the root (`:126-136`).
- **Overlays** are iterated in **reverse** entry order, so the last entry gets the highest priority. All of them are loaded before the root itself (`:139-153`). The inclusion test uses `min_format/max_format` (≥1.21.9) or `formats`/`supported_formats` (`RES/pack/PackMeta.java:60-96`).
- `PackVersion` parsing accepts a number, `"maj.min"`, a float such as `69.1`, or `[maj, min]`. A missing minor defaults to 0 for min and `i32::MAX` for max (`RES/pack/PackVersion.java:73-110`). `VersionRange` accepts an int, `[a, b, ...]`, or `{min_inclusive, max_inclusive}` (`PackMeta.java:107-147`).

**ResourcePack.loadResources** (`RP/ResourcePack.java:114-156`)
1. For each root, these load in parallel (`:158-291`):
   - `assets/*/atlases/**.json`
   - `assets/*/blockstates/**.json`
   - `assets/*/entitystates/**.json`
   - `assets/*/models/**.json`
   - `assets/*/textures/colormap/**.png`
   - `assets/*/blockColors.json`
   - `assets/*/blockProperties.json`
   - Keys come from the path: namespace is segment 1, the value is segment 3 onward with the extension stripped (`RES/ResourcePath.java:78-90`).
2. Extension `loadResources`. The extension registry is empty in core; it is an addon hook.
3. `collectUsedTextureKeys`: every non-reference `TextureVariable` in **every** model, item models included, plus `bluemap:block/missing` (`:321-334`).
4. Textures are loaded per root through the merged `minecraft:blocks` atlas, filtered to the used keys (`:137-141`).
5. Bake (`:293-319`) runs these steps in order:
   - atlas bake (unstitch and paletted permutations)
   - `model.optimize` (pre-resolve texture refs)
   - `applyParent` on all models
   - `calculateProperties`

**DataPack** (`RES/pack/datapack/DataPack.java:85-139`)
- Loads `data/*/dimension_type/**.json` → `DimensionTypeData` and `data/*/worldgen/biome/**.json` → `DatapackBiome`. The biome key uses value position 4, which skips `worldgen/biome` (`:112`).
- Also loads `data/*/defaultBlockstates.json`.
- Bake adds the built-in overworld, overworld_caves, the_nether and the_end dimension types if they are absent (`:132-139`), then builds the `LegacyBiomes` int→key table (`C/world/mca/chunk/LegacyBiomes.java`).

## 2. Blockstates: variants and multipart

**Variants** (`RP/blockstate/Variants.java`)
- Each key is parsed into a condition (`:88-114`):
  - `""`, `"default"` and `"normal"` become ALL, which is the default variant.
  - `a=b,c=d` becomes an AND of `Property`.
  - A malformed key becomes NONE and is dropped.
  - `__comment` keys are skipped.
- Resolution (`:31-49`): the first matching VariantSet in file order wins. If none matches, the default variant is used; if there is no default, `variants[0]`. **Falling back to `variants[0]` is not vanilla behaviour.**

**Multipart** (`RP/blockstate/Multipart.java`)
- Every part whose `when` matches is applied (`:20-26`). A part without `when` is ALL.
- `when` grammar (`:65-106`):
  - `{prop: "a|b", ...}` is an implicit AND.
  - `"OR": [..]` and `"AND": [..]` nest recursively.
  - Values may be JSON booleans, which are stringified.
- An empty `when: {}` calls `and()` with zero args and throws, so the whole blockstate fails to load.

**Matching** (`RP/blockstate/BlockStateCondition.java`)
- Keys and values are lowercased at parse time. A property missing from the state means no match.
- `And` fast-exits when the state has fewer properties than the condition has distinct keys (`:57-82`).

**VariantSet** (`RP/blockstate/VariantSet.java`)
- Accepts either an object or an array.
- The weighted pick is deterministic by position (`:49-63`):
  `h = x*73438747L ^ y*9357269L ^ z*4335792L; f = ((h*(h+456149)) & 0xFFFFFF) / 2^24`.
  Then `selection = f*totalWeight`, subtracting each weight until `selection <= 0`. This uses Java long wrapping arithmetic, so Rust must use `wrapping_mul` on i64.

**Variant** (`RP/blockstate/Variant.java`)
- Fields: `model`, `x`, `y`, **`z`** (a BlueMap extension), `uvlock`, `weight=1`, `renderer` (BlueMap extension, default `bluemap:default`).
- Model paths are lowercased via `ResourcePath(String)`.
- The transform matrix is `T(-.5) · rotateYXZ(-x,-y,-z) · T(.5)` (`:43-50`).
- Renderer types are `bluemap:default|liquid|missing` (`C/map/hires/block/BlockRendererType.java:37-39`). An unknown renderer logs a warning and falls back to default (`RES/adapter/RegistryAdapter.java`).

**Caching**
- The world-`BlockState` → blockstate-file lookup uses a Caffeine cache: 10k entries, 1 min expire-after-access (`ResourcePack.java:105`, `C/util/Caches.java:48-52`).
- **Condition matching and the weighted pick run again for every block** (`C/map/hires/block/BlockStateModelRenderer.java:83-101`).
- `BlockProperties` per state is cached the same way (`ResourcePack.java:356-386`).
- `ResourcePath.getResource` memoises the resolved object inside the path object (`ResourcePath.java:69-72`).
- Unknown block: the `getBlockState` lookup returns null and nothing is rendered. `bluemap:missing` is used by the renderer layer.

## 3. Block models

**Model** (`RP/model/Model.java`)
- Fields: `parent`, `textures: Map<String, TextureVariable>`, `elements[]?`, `ambientocclusion: Boolean?`.
- `display` and `gui_light` are ignored.
- **Parent merge** (`:93-117`):
  - Applied recursively, child-first, with `parent` nulled early to guard against cycles.
  - AO is inherited when the child's value is null.
  - Parent textures are copied when the key is absent.
  - Elements are deep-copied from the parent **only if the child has none**.
  - The merge mutates in place, so the stored model is the flattened result. A missing parent (e.g. `builtin/generated`) is silently ignored.
- `isAmbientocclusion()` defaults to true (`:157-160`).
- `calculateProperties` (`:125-155`):
  - The first full-cube element (0..16 with all 6 faces) sets `occluding=true`.
  - It sets `culling=true` only if every face's resolved texture exists and has an average alpha of exactly 1.

**TextureVariable** (`RP/model/TextureVariable.java`)
- Accepted JSON forms: a string, or `{ "sprite": "..." }` (1.21.x form; other keys skipped) (`:125-149`).
- `#name` is a reference.
- **Quirk:** a string with neither `:` nor `/` is also treated as a reference (`:160-162`).
- Anything else becomes a `ResourcePath`, lowercased.
- Reference resolution is lazy against the **owning model's** merged texture map, cycle-guarded, and the resolved path is cached in the variable (`:80-97`).

**Element** (`RP/model/Element.java`)
- Fields: `from` / `to` default to 0/16, `rotation`, `shade=true`, `light_emission=0`, and `faces: EnumMap<Direction, Face>`.
- Default UV per face when `uv` is absent (`:101-128`):
  - up: `(fx, fz, tx, tz)`
  - down: `(fx, 16-tz, tx, 16-fz)`
  - north: `(16-tx, 16-ty, 16-fx, 16-fy)`
  - south: `(fx, 16-ty, tx, 16-fy)`
  - east: `(16-tz, 16-ty, 16-fz, 16-fy)`
  - west: `(fz, 16-ty, tz, 16-fy)`

**Face** (`RP/model/Face.java:47-51`)
- Fields: `uv: Vec4`, `texture` (default `bluemap:block/missing`), `cullface: Direction?`, `rotation: i32 = 0`, `tintindex = -1`.
- Direction parsing accepts `bottom` and `top` as aliases (`RES/adapter/DirectionAdapter.java`).

**Rotation** (`RP/model/Rotation.java:39-102`)
- Fields: `origin` (default 8,8,8), `axis` + `angle`, **or** per-axis `x`, `y`, `z`, plus `rescale`.
- A non-zero `angle` overrides x/y/z.
- Matrix: `T(-o) · rotateYXZ(x,y,z) · T(o)`.
- `rescale` scales by `1/max|rotated unit axis|` per axis.

**Tint usage**
- In block rendering, any `tintindex >= 0` gets the **single** block-colour calculator result. The index value is ignored (`C/map/hires/block/ResourceModelRenderer.java:285-295`).
- Lowres colour = `texture.colorPremultiplied × tint`, in premultiplied space (`:321-326`).

## 4. Textures, atlas, animation

**Atlas** (`RP/atlas/*`)
- Every pack's `assets/*/atlases/<name>.json` merges into one `LinkedHashSet<Source>` (`Atlas.java:43-48`). Only `minecraft:blocks` is used.
- **BlueMap's bundled `atlases/blocks.json` adds `{"type":"directory","source":"","prefix":""}`**. That loads every PNG under `assets/*/textures/**`, filtered to the used keys (`core/src/main/resourceExtensions/assets/minecraft/atlases/blocks.json`).
- Source types (`SourceType.java:35-41`):
  - `single`: `resource` plus optional `sprite`.
  - `directory`: `source` plus `prefix`. It looks in every namespace, under `textures/<source>/`.
  - `filter`: no-op.
  - `unstitch`: `resource`, `divisor_x/y`, and `regions[{sprite, x, y, width, height}]`. Regions are cut in bake.
  - `paletted_permutations`: `textures`, `separator="_"`, `palette_key`, `permutations{suffix: key}`. Bake builds a key→palette pixel map, multiplies the alphas, and generates `<tex><sep><suffix>` sprites (`PalettedPermutationsSource.java:91-155`).
  - Unknown types log and do nothing.
- The source type is read through a JsonElement first, then decoded again into its concrete class (`Source.java:115-127`).

**Texture** (`RP/texture/Texture.java`)
- Built from the full PNG including all animation frames (`:141-155`).
- `halfTransparent` is true if **any** pixel has `0 < a < 1`. It is false immediately when the image has no alpha channel (`C/util/BufferedImageUtil.java:36-50`).
- `color` is the mean of premultiplied pixels over the **whole strip**, stored straight (`BufferedImageUtil.java:52-68`; `Texture.java:78`).
- `texture` is the PNG **re-encoded** and base64'd as `data:image/png;base64,...`. The decoded image is kept behind a SoftReference.
- `missing`: key `bluemap:missing`, colour (0.5, 0, 0.5, 1), and an embedded 16×16 PNG (`:46-53`).
- `.png.mcmeta` sits beside the PNG (`Source.java:77-83`).

**AnimationMeta** (`RP/texture/AnimationMeta.java`)
- Only the `animation` object is read: `interpolate`, `width`, `height`, `frametime` (int, truncated from a double) and `frames`.
- A frame is either an int or `{index, time}`. A missing `time` becomes `frametime` (`:71-135`).
- Defaults: width = height = frametime = 1.

**ColorMap** (`RP/texture/ColorMap.java`)
- Loads a 256×256 ARGB image.
- Lookup: clamp `t` and `d` to [0,1], then `d *= t`, `x = (int)((1-t)*255)`, `y = (int)((1-d)*255)`, and read pixel `y<<8 | x` with alpha forced to 0xFF.
- The result is treated as premultiplied (`:51-66`).

## 5. Block colours and block properties

**`assets/<ns>/blockColors.json`** (`RES/BlockColorsConfig.java`)
- Format: `{ "<blockstate string>": value }`.
- Keys accept `id[prop=val,...]` via `BlockState.fromString`.
- `putIfAbsent` per exact key, so the higher-priority pack wins (`:50-67`).
- Value syntax (`:114-149`):
  - `@foliage|dry_foliage|grass|water|redstone`: registry calculator.
  - `#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa`: fixed colour. **The alpha is last, CSS order** (`C/util/math/Color.java:184-199`).
  - A bare integer (a JSON number is fine, because lenient `nextString` stringifies it): fixed colour with alpha forced to 255.
  - Anything else: a colormap key, e.g. `minecraft:colormap/foo`.
  - Default is white.
- Lookup: per block id, the first mapping whose listed properties all equal the state's wins (`RES/BlockStateMapping.java:44-53`). Iteration order comes from a HashMap and is **nondeterministic** when mappings overlap.
- One calculator instance per thread (`ResourcePack.createBlockColorCalculator`).

**Calculator types** (`C/map/hires/block/color/BlockColorCalculatorType.java:38-72`)
- `foliage`: colormap `colormap/foliage` (default `#48B518`), then biome `foliage_color` overlay, then blended.
- `dry_foliage`: colormap `colormap/dry_foliage` (default `#8f5f33`). **It overlays `getOverlayFoliageColor`, not dry: an upstream bug.** Then blended.
- `grass`: colormap `colormap/grass` (default `#52952f`), overlay `grass_color`, then the `grass_color_modifier`, then blended.
  - `dark_forest`: `((c & 0xfefefe) + 0x28340a) >> 1`.
  - `swamp`: SimplexNoise seeded `new java.util.Random(2345)` at `(x*0.0225, z*0.0225)`, giving `<-0.1 ? #4c763c : #6a7039` (`C/world/biome/GrassColorModifier.java:40-49`). **Rust must port both java.util.Random and the noise implementation bit-for-bit.**
- `water`: biome `water_color`, blended.
- `redstone`: `((power+5)/20, 0, 0)`.
- Blending (`BlendedBlockColorCalculator.java:46-91`):
  - The default box is ±2 in x/z and ±1 in y, which is a 5×3×5 = 75 sample average.
  - Samples are summed premultiplied, then `flatten()` divides by alpha.
  - The overlay is premultiplied over the colormap colour. A biome without a `*_color` has a fully transparent overlay, which is a no-op (`C/world/biome/Biome.java:56-60`).

**Bundled blockColors** (`core/src/main/resourceExtensions/assets/minecraft/blockColors.json`)
- Leaves, grass family, water, cauldron and redstone.
- Fixed colours for birch and spruce leaves, banners, shulker boxes, lily pad, and the stem age ramp.

**`assets/<ns>/blockProperties.json`** (`RES/BlockPropertiesConfig.java:50-91`)
- Format: `{ "<state>": { culling, occluding, alwaysWaterlogged, randomOffset, cullingIdentical } }`. Each is a tri-state where absent means UNDEFINED.
- Load order is preserved and the first fit wins.

**Final properties** (`ResourcePack.java:360-386`)
1. Extensions apply first.
2. Config then **overwrites** the defined fields.
3. If culling or occluding is still UNDEFINED, it is filled from the models of the matching variants, evaluated at position (0,0,0). The first variant visited wins (verified against the Java code; an earlier version of this doc said last).
4. UNDEFINED reads as false (`C/world/BlockProperties.java:57-75`).

**Bundled blockProperties**
- `alwaysWaterlogged`: seagrass, kelp, bubble_column.
- `randomOffset`: flowers and grass.
- Glass: `occluding:false, cullingIdentical:true`.
- `ice`: `cullingIdentical`.
- Mushroom blocks: forced to `culling` and `occluding`.

**Waterlogging**
- If `waterlogged=true` or `alwaysWaterlogged`, a second `minecraft:water` model is rendered.
- Its colour is overlaid (`BlockStateModelRenderer.java:70-74`).

## 6. Biomes and dimension types (datapack)

**`DatapackBiome.Data`** (`RES/pack/datapack/biome/DatapackBiome.java:79-102`)
- `temperature` and `downfall` default to 0.5.
- `effects { water_color, foliage_color, dry_foliage_color, grass_color, grass_color_modifier }`.
- Colours accept an int, `"#hex"`, `[r,g,b(,a)]` or `{r,g,b,a}` (`RES/adapter/ColorAdapter.java:49-90`). An int without alpha gets 0xFF.
- `water_color` alpha is forced to 1 in a post-deserialize step.
- The default water colour is `4159204` (`#3F76E4`).
- `grass_color_modifier` is resolved through the registry with the default namespace `minecraft`; an unknown value becomes `none`.

**`DimensionTypeData`** (`RES/pack/datapack/dimension/DimensionTypeData.java`)
- Read with Gson `snake_case` (the `@NBTName` annotations are for NBT): `natural`, `has_skylight`, `has_ceiling`, `ambient_light`, `min_y`, `height`, `fixed_time?`, `coordinate_scale`.
- Built-in fallbacks (`C/world/DimensionType.java:34-69`):

  | Dimension | min_y | height | skylight | ceiling | ambient | fixed_time | scale |
  |---|---|---|---|---|---|---|---|
  | overworld | -64 | 384 | yes | no | — | — | — |
  | nether | 0 | 256 | no | yes | 0.1 | 6000 | 8 |
  | end | 0 | 256 | yes | no | — | 18000 | — |

**`data/<ns>/defaultBlockstates.json`**
- Format: `{id: "id[props]"}`.
- Used when a chunk palette entry has no `Properties` (`C/world/mca/data/BlockStateDeserializer.java:83-96`).

## 7. Entity states

- `assets/<ns>/entitystates/<entity>.json` contains `{ "parts": [ { model, position: [x,y,z], rotation: [x,y,z], renderer } ] }` (`RP/entitystate/EntityState.java`, `Part.java:42-72`).
- Lookup is by exact entity id, and the code returns early when it is missing (`C/map/hires/entity/EntityModelRenderer.java:49-50`). Models are ordinary block-model JSON.
- Part transform: `rotateYXZ(-r) · T(pos)`. Renderer types: `bluemap:default|missing`.
- **No vanilla entitystates ship.** Only `bluemap:missing` is bundled.
- Block entities (chest, signs, beds, banners, skulls, shulkers, decorated pot, conduit, copper golem statue) are covered by **blockstate overrides** in `resourceExtensions` pointing at hand-made `minecraft:entity/...` models. There are about 199 model JSONs, split by version overlay into `mc1_15`, `mc1_17`, `mc1_20_3`, `mc1_21_9`, `mc26_1`, `beds` and `signs`.
- The overlays are gated by `pack.mcmeta` `min_inclusive` / `max_inclusive`, e.g. beds `≤85` and signs `≤86` (`core/src/main/resourceExtensions/pack.mcmeta`).
- Liquids are `{"renderer":"bluemap:liquid","model":"block/lava"}`.

## 8. JSON quirks a serde port must replicate

- **Gson lenient** (`RES/adapter/ResourcesGson.java:45-48`, and `setLenient` in the config loaders):
  - `//`, `/* */` and `#` comments
  - unquoted or single-quoted names and strings
  - trailing or double commas, which produce nulls in arrays
  - `;` as a separator
  - NaN
  - Use `json5`/`jsonc`-style pre-sanitising, or a tolerant hand-rolled tokenizer.
- **Type coercions:**
  - `nextString` accepts numbers (blockColors ints).
  - `nextDouble` / `nextInt` accept numeric **strings**.
  - Booleans in multipart `when`.
  - Implement serde `deserialize_with` helpers: `string_or_number`, `num_or_string`.
- **Field naming:** `LOWER_CASE_WITH_UNDERSCORES` (`lightEmission` → `light_emission`, `paletteKey` → `palette_key`). `ambientocclusion`, `tintindex` and `uvlock` are already lowercase. Use `#[serde(rename_all="snake_case")]`.
- Unknown fields are ignored. Missing fields keep their Java field initialisers, so use `#[serde(default = ...)]` everywhere. Explicit `null` keeps the initialiser too for objects; ColorAdapter maps `NULL` to (0,0,0,0).
- `__comment` keys are skipped in variants and `when`.
- `Key` parsing: no `:` (or `:` at index 0) means the `minecraft` namespace. **`ResourcePath(String)` lowercases**, while `Key` does not (`ResourcePath.java:47-49`, `C/util/Key.java`). Registry keys (renderer, grass modifier) default to the `bluemap` / `minecraft` namespace respectively.
- PostDeserialize hooks: Variant matrix, Element default UVs, Rotation matrix, Part matrix, biome water alpha. In Rust, do these in a `bake()` step after serde.
- Per-file failure is swallowed with a debug log (`ResourcePool.java:81-83`). One bad JSON never aborts the load.
- Version-tolerant shapes:
  - `PackVersion`: number, string or array.
  - `VersionRange`: int, array or object.
  - `TextureVariable`: string or `{sprite}`.
  - `VariantSet`: object or array.

## 9. Output written for the webapp

**`<map>/textures.json[.gz]`** (`C/map/TextureGallery.java`, written by `C/map/BmMap.java:108-111, 211-217`)
- A JSON **array** in which the index is the material id used by hires tiles. Gson IDENTITY naming, nulls omitted. Each element:
  ```json
  {"resourcePath":"minecraft:block/stone","color":[r,g,b,a],"halfTransparent":false,
   "texture":"data:image/png;base64,...","animation":{"interpolate":false,"width":1,"height":1,"frametime":2,"frames":[{"index":0,"time":2}]}}
  ```
- `color` is a straight float RGBA. `animation` is present only for animated textures; `frames` is omitted when null.
- **ID assignment** (`TextureGallery.java:76-88`):
  - The existing file is loaded first, so ids stay stable across restarts.
  - Then `bluemap:block/missing` is put first, which gives id 0 on a fresh gallery.
  - The pool is sorted with opaque textures (`a==1`) before translucent ones, then by key string. New keys get `nextId++`.
  - Missing slots are filled with `Texture.MISSING`.
- Unknown key at render time gives 0 (`:61-65`).
- The file is rewritten at map load. A purge resets ids (`BmMap.java:219-222`).
- Client usage (`common/webapp/src/js/map/Map.js:312-377`):
  - `color[3]==1` means opaque and mipmapped.
  - `halfTransparent` decides whether the material is transparent.
  - Animation uses a frame strip with `frametime*50` ms.
- Map `settings.json` is out of scope here (BmMap `saveMapSettings`).

## 10. Rust plan

**Crates**
- `serde`, `serde_json` with a lenient pre-pass (`json5` or a custom comment/trailing-comma stripper; check that the perf is acceptable).
- `zip` for jars and packs (read-only, random access by entry name).
- `png` or `image` for decode/encode, `base64`, `sha1`, `ureq` or `reqwest` (blocking) for the manifest and jar.
- `rayon` for parallel parsing, `ahash`/`rustc-hash`, `lasso` or a custom interner, `glam` for matrices.

**VFS abstraction**
- Model a `PackSource` enum over `Dir(PathBuf)` and `Zip(Arc<ZipArchive>)`, with list and walk on prefixes. Index each zip's central directory once into a `HashMap<path, idx>`.
- Expand fabric nested jars, `data/*/datapacks/*` and overlays into a **flat ordered `Vec<PackSource>`** up front. Every pool is then a single pass of "first insert wins", which can be parallel per kind and merged in priority order.

**Interning**
- Use `ResKey(u32)` interned from `"ns:path"`, with a separate interned namespace. Lowercase at intern time wherever Java uses `ResourcePath`.
- Pools become `Vec<T>` indexed by a dense id, plus `HashMap<ResKey, Id>`. This replaces the per-path memoised `getResource` mutation.

**Model baking (precompute; do not resolve lazily)**
- Topologically flatten parents, then resolve every texture variable to a `TextureId` (or `MISSING`) per model.
- Default UVs, rotation matrices and properties are computed once.
- Store `BakedModel { elements: SmallVec<BakedElement>, ao: bool, culling, occluding }` with `BakedFace { uv: [f32;4], tex: TextureId, cull: Option<Dir>, rot: u8, tint: bool }`.
- Only bake models reachable from blockstates and entitystates, and only load the textures they use. Java loads every item model's textures, so this is a startup and memory win. **Keep `textures.json` ids stable relative to the existing file anyway.**

**Blockstate resolution cache**
- Key it by world palette state id; the chunk reader already produces interned states.
- Precompute `Vec<&VariantSet>`: one entry for Variants mode, every matched part for Multipart. Store it in a `DashMap` or a sharded, grow-only `Vec` indexed by global state id.
- Only the weighted pick remains per block, and only when `variants.len() > 1`. Store `total_weight` and prefix sums.
- `BlockProperties`: compute in the same pass and pack it into a `u16` bitfield of 5 tri-states.

**Textures**
- Decode the PNG once and compute `half_transparent` and the average colour (premultiplied mean) in a single pass. Use `rayon` across textures.
- Write `textures.json` with the original PNG bytes, re-encoding only when the source isn't 8-bit RGBA or was generated (unstitch, paletted). Java always re-encodes; the client doesn't care.
- Keep decoded pixels only for paletted/unstitch inputs, then drop them. This replaces the Java SoftReference.

**Colour**
- Port `Color` as a `#[derive(Copy)] struct { r, g, b, a: f32, premul: bool }` with the same add, overlay, flatten and multiply semantics. Keep f32 to match Java rounding in lowres output.
- Blending costs 75 biome lookups per tinted block. Cache the per-column biome colour within the chunk neighbourhood, or blend in a precomputed per-chunk 2D colour grid (x/z) × 3 y-levels.

**Parity tests**
- Golden tests on vanilla jars:
  - Diff `textures.json` (keys and order, colours within 1e-6, halfTransparent).
  - Variant resolution for every state in a sample world.
  - The `VariantSet` hash and the swamp noise against values captured from Java.
- Known upstream bugs to decide on (match them or fix them):
  - `dry_foliage` uses the foliage overlay.
  - Empty multipart `when` drops the whole file.
  - The Variants fallback to `[0]`.
  - The `Model(…, boolean ambientocclusion)` constructors ignore the flag (`Model.java:65-79`).
