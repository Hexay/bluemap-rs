# 11 — Perf audit: storage, web serving, webapp loading (disk + bandwidth)

This is a static audit of BlueMap 5.27. Nothing was built.
- Source: `src/BlueMap` scratch clone.
- Path prefixes used below:
  - `core/` = `core/src/main/java/de/bluecolored/bluemap/core/`
  - `common/` = `common/src/main/java/de/bluecolored/bluemap/common/`
  - `web/` = `common/.../common/web/`
  - `js/` = `common/webapp/src/js/`
- Measurements cover the 14-map corpus in `bluemap_reverse/work/bluemap/*/web/maps`:
  - 3686 files, 222.3 MB, 800 dirs.
  - The fresh `scratchpad/run/web/maps/structures` render is **byte-identical** to the corpus `structures` tiles (same md5 over all tiles, 81,842,369 B), so the renderer is deterministic.
- Builds on [04](04-storage-web.md) (contract) and [09](09-storage-experiment.md) (PRBM codecs, BMQ1, dedup, lowres WebP). This doc doesn't redo those results.
- Scripts: `docs/storage-exp/{inventory,lowres_audit,lowres_dense,png_filters,textures_audit}.py`. Output is in `docs/storage-exp/out/`.

Constraint: drop-in compatibility with the upstream webapp. Each item is tagged:
- **C (compatible):** server or storage only; same format and webapp. Bytes may change, but the format doesn't.
- **F (format or webapp change):** needs a changed webapp, or bytes that the unchanged webapp can't read.

## Ranked wins

| # | Win | Tag | Measured / estimated effect |
|---|---|---|---|
| 1 | **ETag + If-None-Match → 304 on map data** (tiles, map `settings.json`, `textures.json`) | C | Every browser reload refetches everything. With `useCookies`, which is the default, reload calls `clearTileCache` (`js/BlueMapApp.js:698-701`). The server has no validators (`web/MapStorageRequestHandler.java:57-152`), so each `no-cache` request is a full 200. Default view: 49 hires tiles × 70 KB avg gz = **~3.4 MB**, plus `textures.json` 588 KB, plus up to 243 lowres requests. With ETags this drops to ~290 × ~200 B ≈ **60 KB**. |
| 2 | **Compress static webapp assets** (precompressed `.gz`/`.br` and `Content-Encoding`) | C | `FileRequestHandler` sends everything identity. The JS bundle is 1,201,901 B; gz6 gives 305,182 (0.25×) and br11 gives 249,262 (0.21×). That saves **~0.95 MB per cold visit**. The font is 124 → 57 KB br. |
| 3 | **zstd/br for PRBM, negotiated, with gzip fallback** | C | From 09: zstd-19 is 0.63× and br-11 is 0.65× of stored gzip. Disk, bandwidth and a 49-tile view (3.4 → ~2.2 MB) all shrink by about 37%. Upstream already passes `zstd` storage through when the browser advertises it (`web/MapStorageRequestHandler.java:119-124`). |
| 4 | **Never write unchanged data** (hash before write: hires, lowres, rstate, settings, markers) | C | Write amplification only; disk usage doesn't change. Details in §1.4. Lowres tiles are re-encoded on every debounced save and LOD cascade. rstate is dirtied even when nothing rendered. |
| 5 | **Lowres PNG at zlib 9** (keep filter None) | C | 0.830× (LOD1), 0.897× (LOD2), 0.932× (LOD3), measured on all 445 tiles. PIL `optimize` is **worse** on dense tiles (1.05×), and filter None is already best for 416/445. Lowres is only 1.5% of disk, but it is a large share of per-view requests. |
| 6 | **Don't store fully transparent lowres tiles or empty hires tiles** (serve 204) | C* | 243/333 LOD1 PNGs have an all-transparent colour half: 693 KB, 995 KB on disk, 73% of LOD1 files. 46 hires tiles have 0 vertices. *The webapp already treats 204 as "absent". Height lookup on transparent tiles would change slightly. This is mostly a void-world fixture effect; real worlds are unmeasured. |
| 7 | **textures.json dedup** (disk), plus shared-URL redirect (bandwidth) | C / C? | All 14 copies are byte-identical (1 distinct): 8.23 MB on disk. Each map switch downloads another 588 KB under a new URL. A 302 to a content-hash URL would let the browser cache share it; `fetch` follows redirects, but this needs a browser test. br11 alone gives 0.87×. |
| 8 | **Live JSON: gzip, ETag/`no-cache`, rounded doubles, SSE encoded once** | C | players.json is ~230 B/player/s when polling. 20 players means 4.6 KB/s, **16.5 MB/h per viewer**. markers.json can reach hundreds of KB every 10 s and is sent uncompressed with `no-store` (`web/JsonDataRequestHandler.java:48-53`). |
| 9 | **HTTP framing**: `Content-Length`, `TCP_NODELAY`, 64 KiB copies, no per-request log flush | C | CPU and latency, not bytes. Covered in §2.4. |
| 10 | **Parse q-values in `Accept-Encoding`** | C | `gzip;q=1.0` currently fails to match, so the response goes out uncompressed: about **17× bigger** PRBM (`web/http/HttpHeader.java:65-88`). |
| 11 | **BMQ1 compact hires encoding** | F | 7.5× smaller than gzip PRBM (09). |
| 12 | **Lowres view radius scaled per LOD** | F | `js/map/Map.js:247` uses `lowresViewDistance / tileSize` for every LOD. The default is 4 → 9×9 = **81 tiles per LOD, 243 requests**, and LOD3 covers ±50k blocks. Scaling the radius gives ~81 + 9 + 9 = 99 (−59%). On large worlds that is up to ~290 MB less GPU texture (2 MB per 501×1002 RGBA tile). |
| 13 | **Webapp revalidation bug**: hires URLs never added to `revalidatedUrls` | F | `js/util/RevalidatingFileLoader.js` never calls `.add`; only the texture loader does (`js/util/RevalidatingTextureLoader.js:66`). After a reload, every hires load for the rest of the session is `no-cache`, including tiles you scroll back to. #1 turns these into 304s server-side. |
| 14 | **Idle render loop** (~20 fps while static) | F | `js/MapViewer.js:289-308`. Costs client CPU and GPU, not bandwidth. |
| 15 | **PRBM decode in a worker; drop CPU copies after upload** | F | ~1.21 MB raw per tile is held twice (JS and GPU), about 119 MB for 49 tiles. |
| 16 | **Lowres as WebP-lossless** (under the same `.png` URL, or a new URL) | F (C?) | 0.33× (09) over all tiles, 0.51× on the 12 densest. Browsers sniff image bytes, so the unchanged webapp would probably decode it. Not verified, and it breaks third-party `.png` consumers. Counted as F. |

Not worth pursuing:
- PRBM gzip-9 (0.99×), zstd dictionaries, and dedup (see 09).
- Re-encoding the textures.json PNGs: 1.543 → 1.492 MB, −3%.
- rstate size: 58 KB gz for the whole corpus.
- 4 KiB cluster overhead: +3.5% overall. 628/3686 files are under 4 KiB, mostly lowres and rstate.

## 1. Storage (disk)

### 1.1 Inventory (`inventory.py`)
| category | n | bytes | 4K-disk | files <4K |
|---|---|---|---|---|
| hires `.prbm.gz` | 2996 | 210.76 MB | 216.97 MB | 73 |
| lowres 1/2/3 | 333/56/56 | 2.67/0.46/0.17 MB | 3.15/0.54/0.26 MB | 245/31/48 |
| textures.json.gz | 14 | 8.23 MB | 8.26 MB | 0 |
| rstate tiles/chunks/regions | 77/56/56 | 18/35/5 KB | 315/229/229 KB | all |
| settings/markers/players json | 14 each | 5 KB / 28 B / 28 B | 57 KB each | all |
| **total** | 3686 | 222.35 MB | 230.13 MB (+3.5%) | 628 |

Hires vertex-count distribution:

| vertices | tiles | gz bytes |
|---|---|---|
| 0 | 46 | 3.7 KB |
| ≤600 | 11 | 11 KB |
| ≤6k | 26 | 125 KB |
| ≤60k | 2457 | 146 MB |
| ≤600k | 456 | 64 MB |

Near-empty hires tiles are negligible in bytes. Hires is 95% of disk, so PRBM encoding (09) is the only big disk lever.

### 1.2 PRBM writer
- Non-indexed, 87 B/triangle: f32 position/uv, plus face-constant normal, colour and light written 3× (`core/map/hires/PRBMWriter.java:39,70,88-240`). 09 quantifies the redundancy.
- Bytes are written one at a time through `CountingOutputStream.write(int)` into an 8 KiB `BufferedOutputStream` and then into `GZIPOutputStream` at Deflater level 6 with a 512 B buffer (`core/storage/compression/Compression.java:32-36`, `BufferedCompression.java:47-53`). The file stream underneath is unbuffered, so there are many small syscalls. CPU only.
- Empty models are still written as header + names + `-1` (`PRBMWriter.java:246,275`). `HiresModelManager.java:89-90` has no size check.
- `ArrayTileModel` grows ×1.5 up to 1M faces, about 98 B/face → ~98 MB per render thread worst case (`core/map/hires/ArrayTileModel.java:37-38,485-507`).

### 1.3 Lowres PNG (`lowres_audit.py`, `lowres_dense.py`, `png_filters.py`)
- `ImageIO.write(texture,"png",out)` with no params (`core/map/lowres/LowresTile.java:96`).
  - Measured on every file: 501×1002, 8-bit RGBA, no interlace, zlib FLEVEL=1 (JDK default, roughly level 4), **filter None on all 445,890 rows**, no ancillary chunks.
  - RGB under alpha 0 is already zeroed: 0–1 hidden pixels per LOD.
- Same-format headroom:

  | encoder | ratio vs stored |
  |---|---|
  | zlib 9 + filter None | 0.832 |
  | zlib 9 + best fixed filter | 0.830 |
  | PIL optimize (09) | 0.92 |
  | PIL optimize, 12 densest tiles | 1.05 |

  oxipng/zopfli weren't available. I'd expect at most ~0.75. The meta half (height and light) is 25% of the bytes on dense tiles.
- 09's TYPE_INT_ARGB note still applies: the meta half's alpha is always 0xFF where set (`LowresTile.java:68-71`).
- Rewrite pattern (CPU and write amplification):
  - Every `set()` dirties the tile even when the pixel didn't change, and edge pixels also dirty neighbours (`core/map/lowres/LowresLayer.java:199-227`).
  - Tiles are flushed at 200 pending, on the debounced save 15 s after each region (`core/map/BmMap.java:159-172`), and every 10 min (plugin) or 2 min (CLI).
  - Each LOD save cascades into the next LOD (`LowresLayer.java:150-194`).
  - A 500-block tile spans about 1–4 regions, so it is re-encoded several times per full render.
  - Heap use is ~2 MB per tile, with up to 200 pending + 1000 soft-cached per layer (`LowresLayer.java:51,87-91`).

### 1.4 Write amplification (no content check anywhere)
- **rstate:** every processed tile gets `renderTime=now`, including action NONE (`common/rendermanager/WorldRegionUpdateTask.java:227-233`). So every tile-state cell an update pass touches is rewritten. Region-state is dirtied on every region task (`:254`).
  - On disk this is tiny: 189 files, 58 KB gz, 5.0 MB raw. A chunk-state cell is `int[16384]`, 64 KiB raw.
- **settings.json and markers.json** are rewritten on every `save()` (`BmMap.java:185-186,224-244`).
- **textures.json** is rewritten on every map load (`BmMap.java:110-111`).
- **players.json** is rewritten every `write-players-interval` when that is enabled (`common/plugin/Plugin.java:636-655`).
- Rust fix: keep a hash per tile in memory or rstate, and skip the write when it is unchanged. Format-neutral.

### 1.5 textures.json (`textures_audit.py`)
- 2148 entries, 2,500,763 B raw, of which base64 is 2,126,472 B (85%). 1,557,267 B of PNG, 107 animated.
- 22 duplicate images: 2126 unique of 2148.
- Stored gzip 587,926 B; zstd19 525,657 (0.89×); br11 512,042 (0.87×).
- **All 14 maps hold identical bytes.** The file depends only on the resource pack and the used textures; these fixtures share one pack.
- `Texture.java:151-153` re-encodes each PNG through ImageIO. A re-optimise saves only 3%.

### 1.6 File and SQL storage
- **File writes:** atomic `.filepart` + move costs ~5 metadata syscalls per write, no fsync (`core/util/FileHelper.java`). The digit-split dirs give 800 dirs for 3686 files, about 22%. Format fixed (C: keep).
- **SQL writes:** the whole blob is buffered and then `toByteArray`-copied (`core/storage/sql/SQLGridStorage.java:51-53`). One upsert and one transaction per tile, no batching.
- **SQL reads:** `getBytes` returns a full `byte[]` (`AbstractCommandSet.java:149,218`).
- **SQL compat wins:** batch writes in multi-row transactions, and stream reads with `Bytes`. Per-row overhead is a fixed ~30–40 B over the blob, which is negligible against 70 KB tiles.
- **SQL caveat:** `compression` isn't in the PK, so rows from an old compression become orphans.

## 2. Web server (bandwidth and CPU)

### 2.1 Caching headers
- Default `additional-headers` adds `Cache-Control: public, max-age=86400, stale-if-error=604800` and `CDN-Cache-Control: max-age=60` to every response that doesn't set its own (`webserver.conf:26-28`, `web/BlueMapResponseModifier.java:62` `setHeaderIfAbsent`). That includes tiles, 204s and errors.
  - Within 24 h, a normal navigation is served from the HTTP cache.
  - After 24 h there is **no validator**, so it is a full re-download.
- Reload with `useCookies`, the "update map" button, and SSE-forced tiles all send `no-cache`. All of them get full 200s (#1).
- The static handler has ETag and Last-Modified, but with problems:
  - The ETag is unquoted, so it is invalid per the RFC.
  - `If-None-Match` is compared by exact string.
  - `If-Modified-Since` is checked before the ETag (`web/FileRequestHandler.java:113-139`).
- **Rust:**
  - Quoted strong ETag from mtime+size, or from a content hash for SQL.
  - Correct precedence.
  - `no-cache` revalidation keeps working with the unchanged webapp.
  - Keep the 24 h `max-age` default for compatibility; operators can configure it.

### 2.2 Compression negotiation (`web/MapStorageRequestHandler.java:116-152`)
- Passthrough is good: stored gzip with an accepting client gives the raw bytes.
- Waste:
  - NONE-stored JSON (map `settings.json`, assets, storage live JSON) is decompressed and re-gzipped into a `ByteArrayOutputStream` on every hit, then copied again. Nothing is cached.
  - The static webapp is never compressed (#2). Live JSON is never compressed.
- Rust:
  - Precompress or cache the encoded variants keyed by mtime/hash.
  - Negotiate br/zstd/gzip with q-values.
  - Keep `.gz` URL handling and raw passthrough.

### 2.3 Live data
- `LiveDataSupplierBroadcaster` caches for 1 s (players) and 10 s (markers), re-serialises in full, and compares by `String.equals`. The refresh runs under `synchronized` on the request thread (`web/LiveDataSupplierBroadcaster.java:98-109`).
- SSE broadcasts only on change, but sends the full payload, uncompressed. Each connection re-encodes `"data: "+line` (`web/SseConnection.java:116-134`).
- Players carry full `Double.toString` precision (`common/live/LivePlayersDataSupplier.java:72-90`). Rounding to 2–3 decimals is compatible and cuts ~30%; this is an estimate, as no live data exists in the corpus.
- Compatible SSE compression is possible: a gzip stream with sync flush per event.
- Diffs would need a webapp change.

### 2.4 Framing, threads, logging
- Responses are always chunked, with no `Content-Length` (so no download progress) (`web/http/HttpResponseOutputStream.java:41-67`).
- The body is copied 1 KiB per chunk (`web/http/HttpResponse.java:72-81`).
- Headers are flushed separately and `TCP_NODELAY` is never set, so delayed-ACK stalls are possible.
- Request lines are read one char at a time (`web/http/HttpRequestInputStream.java:105-113`).
- Routing tries a linear regex per map.
- HEAD returns 400 (static) or a full body (map). No Range support. Range isn't needed: tiles are small and whole.
- The webserver log is on by default and goes to `FileHandler`, which is synchronized and flushes per record. That means a global lock plus a syscall on every request (`web/LoggingRequestHandler.java:80`).
- Virtual thread per connection, 600 s idle timeout, no connection cap (`web/http/HttpServer.java:47-55`).
- Rust (axum/hyper) fixes all of these by construction. Keep the log format.

### 2.5 sql.php (external-server path)
- `Cache-Control: public,max-age=86400` even on live JSON, no ETag or 304, and the stored codec is passed through without checking `Accept-Encoding` (`common/webapp/public/sql.php:90-96,192,240`).
- A new PDO connection per request, and whole-blob reads.
- It ships with the webapp. Fixing it is C (PHP only), but it is a separate deliverable.

## 3. Webapp (what the unchanged client costs)
- **Requests per view** at the defaults (hires slider 100, lowres 2000, hires tile 32, lowres tile 500, lodFactor 5, lodCount 3):
  - Hires: (2·⌊100/32⌋+1)² = **49** tiles, loaded only when camera distance is under 1000 (`js/MapViewer.js:463-466`).
  - Lowres: (2·⌊2000/500⌋+1)² = **81 per LOD × 3 = 243**, because the radius isn't scaled per LOD (`js/map/Map.js:244-248`). On small worlds most LOD2/3 requests come back 204.
  - About 8 in flight per manager, ~32 in total (`js/map/TileManager.js:141-145`).
  - Requests are not aborted after scrolling away; the result is just discarded (`js/map/TileLoader.js:82`).
- **Cold-load bytes** for `structures`: JS 1.2 MB (0.3 MB if compressed), `textures.json` 588 KB, hires ~3.4 MB, lowres up to 243 PNGs.
  - Dense LOD1 tiles in the fixtures are 17–135 KB. Real-terrain lowres sizes are not measured.
  - `.js.map` (4.9 MB) sits in the webroot. It costs disk only, since browsers fetch it only with devtools open. Rust can omit it (C).
- **Memory:**
  - Hires: PRBM typed arrays are zero-copy views of the response, but three keeps them alive after GPU upload, so ~2× raw per tile. Raw is 1.21 MB/tile avg, so 49 tiles ≈ 119 MB.
  - Lowres: each texture is 501×1002 RGBA = 2.0 MB on the GPU, plus the retained bitmap for `terrainHeightAt`. With 81 resident LOD1 tiles that is ~160 MB GPU.
  - One `ShaderMaterial` per lowres tile (`js/map/LowresTileLoader.js:97`).
  - Marker geometry leaks on update (`js/markers/ShapeMarker.js:440-444`, `LineMarker.js:282-286`).
- **Live:**
  - Polling (players 1 s, markers 10 s) only runs when SSE fails (`js/BlueMapApp.js:421-430,471,489`).
  - Every response is fully parsed. Shape geometry is rebuilt only on change (`js/markers/ShapeMarker.js:101-107`).
- **Correction to a subagent claim:** with the default `additional-headers`, tiles do carry `max-age=86400`. Heuristic-freshness concerns only apply when an operator removes that header.

## Caveats
- The fixtures are small and partly void-world. Lowres emptiness (#6) and the dense-tile PNG numbers are not representative of survival worlds. The hires numbers are closer.
- Everything in §2–3 is static reading. Request counts are computed from the code, not captured. The live-data byte rates are estimates.
- The WebP-under-`.png` compatibility (#16) and the textures redirect (#7) are untested in browsers.

## Reproduce
```
py -3 -I docs/storage-exp/inventory.py        # §1.1, rstate, webapp asset codecs
py -3 -I docs/storage-exp/lowres_audit.py     # §1.3 IHDR/filters/emptiness
py -3 -I docs/storage-exp/lowres_dense.py     # densest-tile re-encode
py -3 -I docs/storage-exp/png_filters.py      # zlib-9 per-filter headroom
py -3 -I docs/storage-exp/textures_audit.py   # §1.5
```
