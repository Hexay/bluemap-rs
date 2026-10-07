# 04 — Storage, Web Server, Backend↔Webapp Contract

Source root `R` = BlueMap clone. Paths below are relative to it. `core/.../storage` = `core/src/main/java/de/bluecolored/bluemap/core/storage`, `common/.../web` = `common/src/main/java/de/bluecolored/bluemap/common/web`, `webapp/js` = `common/webapp/src/js`.

## 1. Storage abstraction

Interfaces (all stream-based, blocking):
- `Storage` (`core/.../storage/Storage.java`): `initialize()`, `map(id) -> MapStorage`, `mapIds()`, `close()`.
- `MapStorage` (`core/.../storage/MapStorage.java:30-114`): `hiresTiles()`, `lowresTiles(lod)`, `tileState()`, `chunkState()`, `regionState()` (GridStorages), `asset(name)`, `settings()`, `textures()`, `markers()`, `players()` (ItemStorages), `delete(progress)`, `exists()`.
- `GridStorage` (`GridStorage.java`): `write(x,z)->OutputStream`, `read(x,z)->CompressedInputStream?`, `delete`, `exists`, `stream()` of existing cells.
- `ItemStorage` (`ItemStorage.java`): `write/read/delete/exists`.
- `CompressedInputStream` (`compression/CompressedInputStream.java`) carries the **still-compressed** bytes + the `Compression` tag; `decompress()` wraps lazily. This is the zero-copy hook: the web layer forwards raw bytes when the client accepts the encoding.
- Asset names are sanitised by `MapStorage.escapeAssetName` (`MapStorage.java:108-112`): `[^\w\d.\-_/] -> _`, `.. -> _.`.

### Items per map and their compression
| Item | Compression | File path (FileStorage) | SQL key (`Key.formatted`) |
|---|---|---|---|
| hires tile (PRBM) | configured (default gzip) | `<root>/<map>/tiles/0/<grid>.prbm<sfx>` | grid `bluemap:hires` |
| lowres tile lod N (PNG) | always NONE | `<root>/<map>/tiles/<N>/<grid>.png` | grid `bluemap:lowres/<N>` |
| tile render state | always GZIP | `<root>/<map>/rstate/<grid>.tiles.dat` | grid `bluemap:tile-state` |
| chunk render state | always GZIP | `<root>/<map>/rstate/<grid>.chunks.dat` | grid `bluemap:chunk-state` |
| region render state | always GZIP | `<root>/<map>/rstate/regions/<grid>.regions.dat` | grid `bluemap:region-state` |
| settings.json | NONE | `<root>/<map>/settings.json` | item `bluemap:settings` |
| textures.json | configured | `<root>/<map>/textures.json<sfx>` | item `bluemap:textures` |
| markers.json | NONE | `<root>/<map>/live/markers.json` | item `bluemap:markers` |
| players.json | NONE | `<root>/<map>/live/players.json` | item `bluemap:players` |
| asset (e.g. playerheads/<uuid>.png) | NONE | `<root>/<map>/assets/<name>` | item `bluemap:asset/<name>` |

Sources: `file/FileMapStorage.java:46-161`, `KeyedMapStorage.java` (SQL keys). Note `.gz` is appended only to hires + textures; rstate files use `.dat` with no `.gz` suffix but ARE gzip.

### Grid path scheme (`file/FileGridStorage.java:111-133`)
Encode `"x"+x+"z"+z`, cut a folder after **every digit**; last segment gets the suffix.
- `x=-12, z=5` → `x-1/2/z5.prbm.gz`; `x=103,z=-7` → `x1/0/3/z-7.prbm.gz`.
- JS twin: `pathFromCoords` (`webapp/js/util/Utils.js:50-82`) yields the identical string without suffix. Rust must reproduce it byte-for-byte (also used to parse: regex `x(-?\d+)z(-?\d+)` after stripping separators, `FileGridStorage.java:49,91-99`).
- Default file root: `bluemap/web/maps` (`common/.../config/storage/FileConfig.java:40`) — i.e. tiles live *inside* the webroot so any static server can serve them.

### Compression (`compression/Compression.java:45-57`)
| id | key | file suffix | impl |
|---|---|---|---|
| none | bluemap:none | "" | passthrough |
| gzip | bluemap:gzip | .gz | java.util.zip GZIP (default) |
| deflate | bluemap:deflate | .deflate | zlib-wrapped Deflater (= HTTP `deflate`) |
| zstd | bluemap:zstd | .zst | airlift zstd frame |
| lz4 | bluemap:lz4 | .lz4 | **lz4-java `LZ4BlockOutputStream`** — proprietary block framing, NOT the LZ4 frame format; Rust needs a custom reader (magic `LZ4Block`) or drop support |

Atomic writes: `FileItemStorage.write` → `FileHelper.createFilepartOutputStream` writes `<file>.filepart` then atomic-moves on close (`core/.../util/FileHelper.java:49-59`; `atomic=true` default, `FileConfig.java:42`).

## 2. SQL schema (`sql/commandset/*`)

Dialects: `common/.../config/storage/Dialect.java:40-52` — mysql/mariadb → `MySQLCommandSet`, postgresql (`SET synchronous_commit = off`), sqlite (`PRAGMA journal_mode=WAL; synchronous=NORMAL; busy_timeout=30000; foreign_keys=ON`). Table prefix default `bluemap_`, must match `[a-z0-9_]{0,32}` (`AbstractCommandSet.java:46`, `SQLConfig.java:66`).

Six tables (MySQL form, `MySQLCommandSet.java:49-157`, all `COLLATE utf8mb4_bin`):
```
{p}map(id SMALLINT UNSIGNED AI PK, map_id VARCHAR(190) UNIQUE)
{p}compression(id SMALLINT UNSIGNED AI PK, key VARCHAR(190) UNIQUE)      -- "bluemap:gzip" ...
{p}item_storage(id INT UNSIGNED AI PK, key VARCHAR(190) UNIQUE)          -- "bluemap:settings" ...
{p}item_storage_data(map SMALLINT, storage INT, compression SMALLINT, data LONGBLOB,
                     PK(map,storage), FKs ON DELETE CASCADE)
{p}grid_storage(id SMALLINT UNSIGNED AI PK, key VARCHAR(190) UNIQUE)     -- "bluemap:hires", "bluemap:lowres/1" ...
{p}grid_storage_data(map SMALLINT, storage SMALLINT, x INT, z INT, compression SMALLINT, data LONGBLOB,
                     PK(map,storage,x,z), FKs ON DELETE CASCADE)
```
- PostgreSQL: `SMALLSERIAL/SERIAL`, `BYTEA`, upsert `INSERT ... ON CONFLICT (map,storage[,x,z]) DO UPDATE` (`PostgreSQLCommandSet.java:50-196`); purge via `CTID IN (SELECT ... LIMIT ?)` (`:265-277`).
- SQLite: `INTEGER PRIMARY KEY AUTOINCREMENT`, `TEXT`, `BLOB`, `STRICT` tables, `REPLACE INTO` (`SqliteCommandSet.java:45-160`); purge via `ROWID IN (...)` (`:270-282`).
- Blob = the compressed byte stream exactly as the file would be on disk. Writes buffer whole blob in memory then one upsert on close (`SQLGridStorage.java:50-55`).
- Reads filter by `compression = ?` too (`MySQLCommandSet.java:217-227`) — a tile stored with a different compression than currently configured is invisible (forces re-render after changing compression).
- Lookup ids (map/compression/storage key) are find-or-create + cached (`AbstractCommandSet.java:53-56, 380-500`).
- **Migrations/versioning: none.** `initializeTables` lists existing tables; if all six exist, return, else run `CREATE TABLE IF NOT EXISTS` (`AbstractCommandSet.java:87-118`). No schema-version table.
- Map delete: paged `DELETE ... LIMIT 1000` loop then delete map row (cascade) (`SQLMapStorage.java:70-91`).

bluemap-rs against real servers (MariaDB 11.8, MySQL 8.4, PostgreSQL 18; `tools/dbs.py`, `tools/accept_sql.py`,
`tests/sql_remote.rs`): schema (`SHOW CREATE TABLE` / `information_schema` + constraints + indexes) identical to the
tables Java 5.28 creates; Java-rendered DBs re-render 0 tiles here and Java's `-r` rewrites no tile of ours; both
webservers serve the same tile bytes. Intentional differences: `max-connections: -1` (Java: unbounded) = 64, opened
lazily; `dialect:` must agree with the URL's driver (Java would run that dialect's SQL over any driver); no wait is
unbounded: 30 s for a connection, 5 min per statement (`STATEMENT_TIMEOUT`; Java has none), and an overrunning
statement's connection is dropped rather than reused (`sql/db.rs`); blobs above
MySQL `max_allowed_packet` fail before sending (`BlobTooLarge`, #694) instead of mid-protocol; JDBC `user`/`password`
are taken from the URL query too and percent-encoded; dead pooled connections (server restart, `wait_timeout`) are
replaced on the next acquire. JSON items (settings/markers) are stored uncompressed and gzipped per request, so their
gzip bytes differ from Java's (decoded bodies equal).

## 3. Web server

Hand-written HTTP/1.1 server (`common/.../web/http/`, ~1.1k LOC):
- `Server.java:39-99`: one NIO `Selector` thread only for **accept**; each accepted socket handed to `HttpServer.handleConnection` → `HttpConnection` on an executor = `Executors.newVirtualThreadPerTaskExecutor()` (`HttpServer.java:47-55`). So: blocking I/O, one virtual thread per connection, keep-alive loop (`HttpConnection.java:52-61`), 10-min SO_TIMEOUT.
- Request parsing: regex request line, headers until blank, supports `Content-Length` and chunked bodies (`HttpRequestInputStream.java:54-131`). Only path (no query) used for routing.
- Responses: **always `Transfer-Encoding: chunked`** when a body exists, else `Content-Length: 0`; headers flushed immediately (`HttpResponseOutputStream.java:41-67`); body copied in 1 KiB chunks (`HttpResponse.java:70-89`). No range requests, no HEAD special-case, no TLS, no HTTP/2.
- Handler chain (`common/.../plugin/Plugin.java:213-273`, CLI twin `implementations/cli/.../BlueMapCLI.java:286-324`):
  `LoggingRequestHandler` → `BlueMapResponseModifier` (adds `Server: BlueMap/<ver>`, replaces 4xx/5xx bodies with text, applies `additional-headers` via setHeaderIfAbsent) → `RoutingRequestHandler`.
- Routing (`RoutingRequestHandler.java:51-85`): regex routes, **last registered wins** (`addFirst`), optional replacement (`$1`). Routes:
  - `.*` → `FileRequestHandler(webroot)` (static webapp)
  - `maps/<quoted id>/(.*)` → `MapRequestHandler` (path rewritten to `$1`)
  - inside MapRequestHandler (`MapRequestHandler.java:64-101`): `.*` → `MapStorageRequestHandler`; `live/sse` → SSE (if enabled); `live/markers\.json` → live JSON (10 s cache); `live/players\.json` → live JSON (1 s cache). Maps without a loaded `BmMap` (e.g. CLI w/o render) get only storage serving (players/markers then come from storage items).
- `FileRequestHandler.java:54-176`: GET only; path traversal guard; dir w/o slash → 303 to `/dir/`; missing → tries `index.html`; `.php` → 403; `If-Modified-Since` (1 s slack) / `If-None-Match` → 304; sets `ETag` (= hex size+pathHash+mtime) and `Last-Modified`; fixed content-type map (json/png/jpg/svg/css/js/html/xml, else text/plain).
- `MapStorageRequestHandler.java:52-152` (map data):
  - Strips leading/trailing `/`; a trailing `.gz` in the URL sets `requestGzipped` (clientDecompression mode).
  - Tile regex `tiles/([\d/]+)/x(-?[\d/]+)z(-?[\d/]+).*` → lod, x, z (slashes removed). lod 0 → hires (`application/octet-stream`), else lowres (`image/png`). **Missing tile → `204 No Content`**.
  - Else switch: `settings.json`, `textures.json`, `live/markers.json`, `live/players.json`, `assets/<name>`; content-type from filename. Unknown → 404; IO error → 500.
  - Encoding negotiation (`:116-152`):
    1. not `.gz` URL and stored compression ≠ NONE and `Accept-Encoding` contains its id (`gzip`/`deflate`/`zstd`/`lz4`) → **raw passthrough** with `Content-Encoding: <id>`.
    2. else if stored ≠ GZIP, not PNG, client accepts gzip → decompress + re-gzip in memory.
    3. else decompress and send identity.
    4. `.gz` URL: GZIP stored → raw; otherwise transcode to gzip. No `Content-Encoding` header (client decompresses via `DecompressionStream`).
  - Sets **no cache headers** itself; default `additional-headers` supply `Cache-Control: public, max-age=86400, stale-if-error=604800` + `CDN-Cache-Control: max-age=60` (`common/src/main/resources/de/bluecolored/bluemap/config/webserver.conf`). No ETag on map data.
- Live JSON (`JsonDataRequestHandler.java:44-55`): `application/json`, `Cache-Control/CDN-Cache-Control/Cloudflare-CDN-Cache-Control/Surrogate-Control: no-store`.
- Logging (`LoggingRequestHandler.java:54-99`): Java format string, args 1 src ip, 2 leftmost X-Forwarded-For, 3 method, 4 path?query, 5 version, 6 status, 7 msg; default `%1$s "%3$s %4$s %5$s" %6$s %7$s`; 5xx logged as warning.
- Config (`webserver.conf`): `enabled`, `webroot` (default `bluemap/web`), `ip`, `port` 8100, `sse-enabled` true, `additional-headers`, `log.file/append/format`.

### External web servers
- Webapp is built by Vite (`base: './'`, `common/webapp/vite.config.js:9`), zipped into `common/src/main/resources/de/bluecolored/bluemap/webapp.zip` (`common/build.gradle.kts:34-46`) and extracted into webroot when `index.html` is missing (`common/.../WebFilesManager.java:106-117`).
- File storage: tiles sit at `web/maps/<id>/tiles/...prbm.gz`; nginx/apache serve them with `gzip_static`-style rules (configs live in the external BlueMap wiki, not in this repo).
- SQL storage: `common/webapp/public/sql.php` — PDO (mysql/pgsql), hardcoded `bluemap_` prefix, joins the 4 tables, maps compression key → `Content-Encoding` (`sql.php:18-24, 90-96`), `Cache-Control: public,max-age=86400`, missing tile → 204, unknown → 404. Same URL scheme; serves `index.html` at `/`. Live data in that setup comes from storage items written every `write-markers-interval` (10 s) / `write-players-interval` (3 s) (`common/src/main/resources/.../plugin.conf:43-55`, `Plugin.java:326-345, 633-652`).

## 4. Webapp contract (every request)

Base = page dir. `mapDataRoot`/`liveDataRoot` default `"maps"`; per map `M = mapDataRoot+"/"+id`, `L = liveDataRoot+"/"+id` (`webapp/js/BlueMapApp.js:319-321`).

| URL | When | Loader / parser |
|---|---|---|
| `settings.json` | startup, no-cache | `BlueMapApp.js:379-390`, defaults merged `:343-374` |
| `lang/settings.conf`, `lang/<lang>.conf` (HOCON) | startup | `common/webapp/src/i18n.js:29,38` (static) |
| `assets/*` (logo, steve.png, poi.svg) | UI | static |
| `<settings.scripts[]>`, `<settings.styles[]>` | startup | `BlueMapApp.js:152-201` |
| `M/settings.json` | map load | `map/Map.js:65,164-276` |
| `M/textures.json` (or `.json.gz` if clientDecompression) | map load | `map/Map.js:66,282-296`, used `:312-` |
| `M/tiles/0/<grid>.prbm` (+`.gz`) | hires tiles | `map/TileLoader.js:64-108` → `PRBMLoader.parse` |
| `M/tiles/<lod>/<grid>.png`, lod 1..lodCount | lowres tiles | `map/LowresTileLoader.js:65-142` |
| `L/live/players.json` | 1 s poll | `BlueMapApp.js:396-410, 454-477`, `markers/PlayerMarkerManager.js` |
| `L/live/markers.json` | 10 s poll | `BlueMapApp.js:479-495`, `markers/NormalMarkerManager.js:46-49` |
| `L/live/sse` (EventSource) | map switch | `BlueMapApp.js:412-452` |
| `M/assets/playerheads/<uuid>.png` | player markers | `BlueMapApp.js:464`, `markers/PlayerMarkerSet.js:81` |

Fetch semantics (`webapp/js/util/RevalidatingFileLoader.js:151-301`): `fetch` with `cache:"no-cache"` for URLs not yet revalidated this session; **only status 200 (or 0) is success** — 204/404 are errors (tile treated as absent). Progress uses `X-File-Size` or `Content-Length`. If `clientDecompression`, body piped through `DecompressionStream("gzip")` (`:258-263`). Lowres PNGs go via `RevalidatingTextureLoader` → `createImageBitmap(blob, {colorSpaceConversion:'none'})`.

### Root `settings.json` (`common/.../WebFilesManager.java:120-150`)
```
{version, useCookies, defaultToFlatView, startLocation|null, resolutionDefault,
 minZoomDistance, maxZoomDistance, hiresSliderMax/Default/Min, lowresSliderMax/Default/Min,
 mapDataRoot, liveDataRoot, clientDecompression, maps:[id...], scripts:[...], styles:[...]}
```
Static file in webroot, written by backend on start (maps sorted by map `sorting`).

### Map `settings.json` (`core/.../map/MapSettingsSerializer.java:40-110`)
```
{name, sorting, hires:{tileSize:[x,z], scale:[1,1], translate:[x,z]},
 lowres:{tileSize:[x,z], lodFactor, lodCount}, startPos:[x,z], skyColor:[r,g,b,a], voidColor:[r,g,b,a],
 ambientLight, skyLight, perspectiveView, flatView, freeFlightView}
```
Vectors as arrays (JS `vecArrToObj`), colors as float arrays. JS defaults: hires tile 32, translate 2, lowres tile 32, lodFactor 5, lodCount 3 (`Map.js:73-82`).

### `textures.json` (`core/.../map/TextureGallery.java:90-102`, `core/.../resources/pack/resourcepack/texture/Texture.java:45-69`)
Array indexed by material id: `[{resourcePath, color:[r,g,b,a], halfTransparent, texture:"data:image/png;base64,...", animation?:{interpolate, frametime, frames?:[...]}}]`. Consumer `Map.js:312-356`, `map/TextureAnimation.js`.

### Hires tile = PRBM (PRWM-derived) — writer `core/.../map/hires/PRBMWriter.java`, parser `webapp/js/map/hires/PRBMLoader.js:112-252`
- Byte 0 version=1; byte 1 flags `0b0_0_0_00111` (non-indexed, LE, 7 attributes); bytes 2-4 valuesNumber (LE 24-bit, = vertex count = triangles*3); bytes 5-7 indicesNumber=0.
- Per attribute: NUL-terminated ASCII name, 1 flag byte (`bit7 type`, `bit6 normalized`, `bits4-5 cardinality-1`, `bits0-3 encoding`: 1=f32, 3=i8, 4=i16, 6=i32, 7=u8, 8=u16, 10=u32), pad to 4, then `cardinality*valuesNumber` values, LE.
- Attribute order/types (`PRBMWriter.java:62-232`): `position` f32x3; `normal` i8x3 normalized; `color` u8x3 normalized; `uv` f32x2; `ao` u8x1 normalized; `blocklight` i8x1; `sunlight` i8x1.
- Then pad to 4 and material groups: repeated `(i32 materialIndex, i32 start, i32 count)` LE, terminated by `i32 -1` (`PRBMWriter.java:242-277`, `PRBMLoader.js:222-237`). Empty model = header+attrs with 0 values + `-1`.
- Served gzip (or other) compressed; browser decodes `Content-Encoding`.

### Lowres tile PNG (shaders `webapp/js/map/lowres/LowresVertexShader.js`, `LowresFragmentShader.js`)
Image is `(tileSize+1) x 2*(tileSize+1)`: top half = RGBA color (alpha 0 = void/discard); bottom half = metadata: `R` = block light (0-15, `metaToLight`), `G,B` = 16-bit height `G*256+B`, two's complement (>=32768 → negative) (`metaToHeight`, `posToMetaUV` offsets v by 0.5). Lod N covers `tileSize*lodFactor^(N-1)` blocks; mesh `PlaneGeometry(tileSize+1)` (`LowresTileLoader.js:55-62,95-129`).

### `live/players.json` (`common/.../live/LivePlayersDataSupplier.java:56-102`)
`{"players":[{"uuid","name","foreign":bool,"position":{"x","y","z"},"rotation":{"pitch","yaw","roll"}}]}` — consumer `markers/PlayerMarkerSet.js:42-89`, `PlayerMarker.js:108-157`. Persisted fallback when no live server: `{}` (`core/.../map/BmMap.java:246-252`).

### `live/markers.json` (`LiveMarkersDataSupplier.java:41-44` = `MarkerGson.toJson(Map<String,MarkerSet>)`, from BlueMapAPI)
`{ "<setId>": {label, toggleable, defaultHidden, sorting, markers:{ "<id>": {type, ...} } } }` (`markers/MarkerSet.js:89-176`). Types and fields read:
- common: `position{x,y,z}`, `label`, `detail`, `link`, `newTab`, `listed`, `sorting`, `minDistance`, `maxDistance`
- `poi`: `icon`, `anchor|iconAnchor{x,y}`, `classes[]`; `html`: `html`, `anchor`, `classes[]`
- `shape`: `shape[{x,z}]`, `holes`, `shapeY`, `lineColor`, `fillColor`, `lineWidth`, `depthTest`
- `extrude`: `shape`, `holes`, `shapeMinY`, `shapeMaxY`, colors, `lineWidth`, `depthTest`; `line`: `line[{x,y,z}]`, `lineColor`, `lineWidth`, `depthTest`
- Colors are `{r,g,b,a}` objects, r/g/b 0-255 (`markers/ShapeMarker.js:120-126`; `borderColor` legacy alias). Rust must replicate MarkerGson exactly — get canonical shapes from the BlueMapAPI repo.

### SSE `live/sse` (`MapRequestHandler.java:73-128`, `SseConnection.java:93-135`)
Headers: `text/event-stream`, `Cache-Control: no-store` (+CDN variants), `X-Accel-Buffering: no`. Events: `tile` data `{"x":..,"y":..,"lod":..}` (y = tile z), `player` data = players.json body, `marker` data = markers.json body. Keepalive comment `:` every 30 s; per-client queue 64, overflow closes client. Multi-line data split into `data:` lines.

## 5. Live refresh model
- Client polling: players every 1000 ms, markers every 10000 ms; back-off to 15 s on failure; polling paused while SSE is open, resumed on SSE error (`MarkerManager.js:59-100`, `BlueMapApp.js:421-430, 468-489`). `setTimeout` chain, not setInterval.
- SSE tile event → force-reload tile if loaded (`BlueMapApp.js:432-443`); forced loads bypass revalidation cache.
- Server: `LiveDataSupplierBroadcaster` caches supplier output for pollInterval (1 s players / 10 s markers) per HTTP request; when SSE clients exist it polls on the scheduler and broadcasts only on change (`LiveDataSupplierBroadcaster.java:62-113`). Tile updates pushed from render listeners (`MapRequestHandler.java:54-57`).
- Storage copies of markers/players written on timers for external-webserver setups (10 s / 3 s).

## 6. Rust plan
- HTTP: **axum + hyper + tokio** (SSE via `axum::response::sse` with `KeepAlive` 30 s comment; `tower-http` for `TraceLayer`, `SetResponseHeader` for additional-headers, `ServeDir` optional). tiny_http lacks async SSE fan-out; avoid.
- Router: `/maps/{id}/live/sse`, `/maps/{id}/live/players.json`, `/maps/{id}/live/markers.json`, `/maps/{id}/{*path}` (storage), fallback = embedded webapp. Match Java edge cases: 204 for missing tile, trailing `.gz` handling, `tiles/<lod>/x..z..` regex accepting anything after z digits.
- Storage trait mirrors Java: `async fn read(&self,..) -> Option<(Bytes|File, Compression)>`. Serving: if `Accept-Encoding` contains stored id → send raw (`Content-Encoding`, real `Content-Length` — Java always uses chunked; Content-Length improves progress). Files: `tokio::fs::File` → `ReaderStream` (or `sendfile` via `tokio`'s copy; true zero-copy needs `hyper` + custom body—not worth it). SQL: blob → `Bytes` directly.
- Transcode fallback: `flate2` (gzip/deflate), `zstd`, lz4 (custom `LZ4Block` decoder over `lz4_flex::block` — or declare lz4 unsupported); `async-compression` for streaming.
- SQL: `sqlx` (mysql/postgres/sqlite, one `Any` pool or per-dialect enum) — keep exact table names/columns/key strings (`bluemap:hires`, `bluemap:lowres/N`, `bluemap:gzip`...) so existing DBs and `sql.php` keep working; `rusqlite` only if sync is preferred for SQLite. Port the 6 `CREATE IF NOT EXISTS` + upsert variants per dialect; add a `{p}meta(schema_version)` only if we diverge.
- File storage: reproduce digit-split paths, `.filepart` + rename atomic writes, suffix table, `rstate/` names, `live/` and `assets/` layout so existing `web/maps` dirs are reusable.
- Webapp as-is: build with `npm ci && npm run build` (Vite, `base:'./'`), embed `dist/` via **rust-embed** (supports `include-exclude`, gives mtime/etag-able hashes, can serve pre-compressed `.gz` variants) or `include_dir`. Write root `settings.json` to disk webroot (it is user-editable and merged), serve disk overrides before embedded assets so users can add `scripts/styles` and custom files.
- Static handler parity: ETag/If-None-Match, Last-Modified/If-Modified-Since, 303 dir redirect, index.html fallback, block `.php`.
- Concurrency: tokio multi-thread; live JSON via `tokio::sync::watch` (latest value) + `broadcast` for SSE fan-out with bounded per-client buffer (drop slow clients like Java's 64-queue).
- Logging: `tracing`; reproduce Java log format arg set (src, XFF, method, path?query, version, status, reason).

## Gotchas
- Missing tile = 204, not 404; webapp treats any non-200 as "no tile".
- SQL reads filter on compression id: changing compression hides old tiles.
- rstate `.dat` files are gzip despite no `.gz` suffix.
- Lowres PNGs are never compressed and never re-gzipped (`MapStorageRequestHandler.java:127`).
- `sql.php` hardcodes prefix `bluemap_`.
- Live JSON must keep `no-store` even if `additional-headers` sets Cache-Control (Java uses setHeaderIfAbsent).
- SSE `tile` event uses key `y` for the tile z coordinate.
