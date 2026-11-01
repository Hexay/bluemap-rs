# What a first load of the real webapp consists of (devbox, 2026-11-01)

One first load and one reload per row, fresh profile, headless Chrome (Playwright chromium-1248), loopback, the 42-tile
view `#real:0:70:0:60:0:0.9:0:0:perspective` of the 4096² world. Byte counts only: runs 1, no timing claims.

Regenerate under `~/bench.lock`: `docs/perf-exp/devbox_web_load_dump.sh` (`view.mjs --dump` + `load_breakdown.mjs`).
Rows: **Java** = Java 5.28 server, Java render, 5.29 static files (`j529`); **rs opt** = `bluemap -w`, optimized + script
(`ro`); `rc` (ours serving Java's render) only where noted. Headline requests match the earlier runs (271/272 without
live data; 146–147 × 200, 124 × 204, 1 cached).

## First load

Wire KB includes response headers. Decoded = after `Content-Encoding`.

| category | n Java / rs | wire KB Java | wire KB rs opt | decoded KB Java | decoded KB rs opt | encoding Java / rs |
|---|---|---:|---:|---:|---:|---|
| webapp JS | 1 | 1182.8 | 298.7 | 1174.5 | 1174.5 | identity / gzip |
| webapp CSS | 1 | 19.4 | 3.8 | 19.0 | 19.0 | identity / gzip |
| HTML | 1 | 1.9 | 1.0 | 1.6 | 1.7 | identity / gzip |
| font | 1 | 122.4 | 64.8 | 121.3 | 121.3 | identity / gzip |
| lang | 2 | 5.9 | 2.7 | 5.3 | 5.3 | identity / gzip |
| client script (JS + WASM) | 0 / 1 | - | 47.5 | - | 116.9 | - / gzip |
| textures.json | 1 | 578.3 | 574.3 | 2442.2 | 2442.2 | gzip / gzip (stored) |
| map settings.json | 2 (1 from cache) | 0.5 | 0.5 | 0.7 | 0.7 | gzip / gzip |
| root settings.json | 1 | 0.6 | 0.6 | 0.4 | 0.4 | identity |
| lowres LOD 1 | 72 × 200 | 10085.1 | 8338.0 | 10003.1 | 8320.9 | identity (PNG) |
| lowres LOD 2 | 16 × 200, 56 × 204 | 1483.3 | 1379.9 | 1461.5 | 1368.4 | identity (PNG) |
| lowres LOD 3 | 4 × 200, 68 × 204 | 115.4 | 108.1 | 103.3 | 97.8 | identity (PNG) |
| hires tiles | 42 | 2453.4 | 162.8 | 42887.9 | 2031.1 (BMQ3) | gzip / zstd |
| live data | 5 (1 × 404: SSE, `-w` has none) | 1.1 | 1.1 | 0 | 0 | gzip |
| other (icons, logo, manifest) | 3 | 60.4 | 59.8 | 59.2 | 59.2 | identity / gzip where it compresses |
| **total** | 276 / 277 | **16110.5** | **11043.5** | | | |

Lowres is 89% of the rs opt load; LOD 1 alone is 75%.

Cache headers:

- Java: everything carries the default `additional-headers` (`public, max-age=86400, stale-if-error=604800`,
  `CDN-Cache-Control: max-age=60`). `ETag` + `Last-Modified` on static files only, no validator on map data.
- rs: the same, plus `ETag` on map data, except the client script (`max-age=31536000, immutable`) and `index.html`
  (`no-cache`). 204s have no `ETag`.

Reload: Java 14698.5 KB, rs 25.7 KB (143 requests with `If-None-Match`; Chrome reports the revalidated ones as 200).

## The 124 × 204s

- Lowres PNGs outside the rendered world: 56 at LOD 2 and 68 at LOD 3, none at LOD 1 or hires. Each LOD asks for the
  same 72-tile square, x −4..4, z −3..4 (`Map.loadMapArea`: the tile radius is `lowresViewDistance / tileSize`, not scaled
  by LOD). The world only fills x −2..1 at LOD 2 and x −1..0 at LOD 3.
- `settings.json` has no map bounds, so the webapp can only find out by asking. Upstream does exactly the same: the
  count and URLs are identical on Java. `Missing tile → 204` is upstream's contract (docs/04), and the webapp treats it
  as "absent".
- Cost, first load: 17.5 KB down (rs; 19.8 KB Java), 70.6 KB of request headers up, median 0.9 ms each on loopback.
  So ~88 KB plus 124 trips through the loader's 8-in-flight limit. Time on a real link was not measured here.
- On reload they go through `fetch` instead of an image load and show as `net::ERR_ABORTED`, canceled, with no response
  recorded; the requests are still sent. Why Chrome reports them that way was not investigated.
- Fixture-specific: on a world larger than the view these become 200s with PNG bodies, so a big map pays more here.

## Why the non-tile totals differ (13657 KB Java vs 10881 KB rs opt: 2776 KB)

| cause | KB |
|---|---:|
| Java's render has larger lowres PNGs (zlib level; `rc`, ours serving Java's files, keeps them: 11607 KB) | 1858 |
| Java sends static files uncompressed: JS 884, font 58, CSS 16, lang 3, HTML 1, other 1 | 963 |
| `textures.json`, headers | 4 |
| the client script, which only rs has | −48 |
| **total** | **2776** |

## Savings on the rs opt first load (estimates, nothing implemented)

Byte numbers come from re-encoding this load's own files. `br`/`zstd` only help where the browser advertises them; here
Chrome did on `127.0.0.1`. From memory it does not on plain remote `http://`, the usual `:8100` setup: check that first.

| # | change | saves (of 11043 KB) | risk |
|---|---|---:|---|
| 1 | Lowres PNG as stored-deflate IDAT under `br` 11 / `zstd` 19, negotiated | ~1950 KB (br) / ~1610 KB (zstd), 15–18% | Medium. The biggest item, but the decoded body is no longer the stored PNG (same pixels, not yet verified in a browser). 2 MB of scanlines per tile: needs caching or pre-encoding, CPU unmeasured. HTTPS only: `gzip` gains nothing (+0.5%) |
| 2 | Precompressed `br` for embedded static files (JS 55, font 9, script 7, CSS 0.4) | ~71 KB | Low. Known bytes at build time, `Vary` already sent; user-replaced webroot files stay on gzip |
| 3 | `textures.json` as `br` 11 / `zstd` 19 | ~74 / ~60 KB | Low–medium. Stored gzip is passed through today, so this needs a cached transcode per map; `.json.gz` URLs stay gzip |
| 4 | Answer 204s in the client script from a bounds hint | ~88 KB, plus 124 requests | Medium. Needs bounds from the server, goes stale while rendering; no-script clients unchanged. Byte gain is small, the possible gain is latency (unmeasured) |
| 5 | Long `max-age` + `immutable` on hashed webapp assets | 0 on first load; ~5 revalidations after 24 h | Low. Must yield to a user's `additional-headers` |

Migrating installs only: **Java-rendered lowres files** cost 1858 KB more than ours until re-rendered, 0 for rs-rendered
maps. A rewrite pass would be low risk (same pixels, ours already differ from Java's bytes); not a serving change.

No gain, or against drop-in:

- Refiltering PNGs: fixed filters are all worse than None (LOD 1: 10.2–13.0 MB vs 8.5 MB). Ours are already at zlib 9
  level, re-deflating at 9 is +0.2%. Zopfli/oxipng not measured.
- Stronger gzip for static files: gz9 saves ~1 KB on the JS.
- Live data: 1.1 KB here (no players on this fixture, so not representative).
- Removing the default `CDN-Cache-Control`, changing the 204 contract or lowering default view distances: these change
  behaviour upstream users rely on.

Compared with upstream, the drop-in rule requires 204 for missing tiles, the same URLs and decodable PNG/PRBM content.
rs already goes beyond it with gzip on static files and validators on map data; every row above keeps those URLs.
