# 08 — BlueMap GitHub issues: what users complain about

Source: all 628 issues (PRs excluded) on BlueMap-Minecraft/BlueMap, #1 (2019-12) – #872 (2026-09), pulled 2026-10-06.
Each issue got one primary category. Only the issue bodies were read (first 1500 chars), not the comment threads, and the
repo has no GitHub Discussions. Most support happens on Discord, which isn't covered here, so treat the counts as a
lower bound. Raw data: scratchpad `issues/all.tsv` (regenerate with `gh api --paginate repos/BlueMap-Minecraft/BlueMap/issues?state=all`).

Engagement = comments + reactions.

## Counts

| Category | Issues | Open | Engagement | Note |
|---|---|---|---|---|
| webapp-ui | 85 | 3 | 260 | JS client — we reuse it, mostly out of scope |
| render-visual | 67 | 3 | 221 | correctness, incl. "map not updating" cluster |
| platform-integration | 59 | 0 | 191 | version bumps, mod/classpath conflicts |
| feature-request | 47 | 6 | 154 | |
| api-markers | 47 | 1 | 107 | |
| mod-resourcepack | 47 | 0 | 96 | |
| storage-sql | 43 | 1 | 183 | **highest engagement per issue among backend areas** |
| live-players | 40 | 2 | 114 | |
| config-setup | 38 | 0 | 95 | world/dimension discovery #1 |
| webserver | 36 | 0 | 98 | |
| world-format-version | 33 | 0 | 87 | |
| perf-speed | 24 | 0 | 90 | |
| question-support | 24 | 0 | 66 | |
| crash-other | 23 | 0 | 94 | |
| memory | 6 | 0 | 44 | |
| not-applicable | 9 | 0 | 11 | |

Top engaged: #80 MySQL storage (55), #635 map not updating (29), #85 CLI crash (27), #220 Forge 1.12.2 (25),
#398 live daylight (22, open), #328 zoom to cursor (22, open), #95 player models (21, open), #271 memory 5→16 GB (20),
#146 sign text (20, open), #244 WebSocket instead of polling (18).

## Headline finding

Explicit speed and memory complaints are **only 30 of 628 issues (~5%)**, and most of them are from 2020–2021,
before BlueMap moved to the PRBM tile format and mutable vectors. The pain users actually report is about the
consequences of running inside the server and of the storage design:

1. **Competing with the game server**: 2–5 GB shared heaps on typical hosts (#480, #541, #444), renders lagging or
   crashing the server (#215, #176, #171, #711), 5 GB used while idle (#565), 5→16 GB growth (#271).
   → Running as a separate process with a fixed memory budget is the strongest selling point, more than raw speed.
2. **Disk size and file count**: renders bigger than the world (#140, #285: 20+ GiB for 400×400), 110 GB world filled
   512 GB (#500), 1.5M files / ~100 GB in days (#701), backups too slow (#217), SQL growing 3→15 GB on an unchanged world (#722).
3. **Maps silently stop updating** — the top render cluster (#635, #786, #618, #796), caused by watcher and scheduler bugs
   (#868 `full-update-interval: 0` kills watchers, #869 hourly instead of daily, #576 watcher NPE, #655).
4. **Thread scaling**: 32 threads 3.5 tiles/s vs 1 thread 3.9 (#72), 12 threads ≈ 1–2 (#592), Nether 8 → 1.1 tiles/s (#56).
5. **Windows file writes**: `.filepart` rename failures (#471, #357, #326, #498, #241, #666), purge AccessDenied (#669).

## Rust rewrite: what it fixes outright

- JVM heap/GC pressure: #183 (immutable vectors → GC OOM), #271, #565 — value types and native memory.
- HTTP parsing bugs in the hand-written server: absolute-form targets (#737), unbounded chunked body → remote OOM
  (#828), null path NPE (#750), loopback accept-then-close (#527) — hyper/axum.
- Classpath/Java-version conflicts (#379, #462, #516, #489 headless ImageIO) — for the core, not the Java shim.
- Bundled SQL drivers (#510); type-safe ordering removes TimSort contract crashes (#203).

## Must NOT replicate (deliberate design points)

| Bug | Issues | Design rule |
|---|---|---|
| Concurrent lowres saves corrupt PNGs | #821, #593 | One writer per tile key; temp+rename |
| Disk-full corrupts tiles, loses render state | #99, #121 | Stop cleanly; never overwrite state on IO error |
| Windows rename-replace fails | #471, #241, #666, #669 | `MoveFileEx(REPLACE_EXISTING)` + retry on sharing violations; close handles before delete |
| Watchers die silently; 0 interval crashes | #868, #576, #869, #655 | Watcher failure is loud + auto-restart; `0` = disabled |
| Idle CPU burn | #799 | Block on fs events (notify crate) |
| Eager task enumeration / startup scan | #711, #583, #58 | Lazy/streamed task creation; never scan on startup path or main thread |
| Re-render on any file touch | #653 | Compare chunk timestamps from region header, not file mtime |
| Thread non-scaling | #72, #592 | Share-nothing per-region workers, no global hot locks |
| Corrupt region → retry loop / log spam | #560, #569, #587 | Skip bad chunk once, log once |
| NPE on missing NBT fields (old chunks) | #521, #563 | All optional fields with defaults; missing light ⇒ degrade, not black |
| Strict model loader crashes worker | #128, #147, #291, #292 | As lenient as vanilla, cycle-safe, per-block failure isolation |
| MySQL-only SQL | #335, #488, #419 | Per-dialect SQL, CI against MariaDB/MySQL 5.7/Postgres/SQLite |
| 25 MB packets > `max_allowed_packet` | #694 | Chunk or cap blob writes |
| Webserver-only mode needs DB write | #749 | Read-only mode, check tables before creating |
| Purge misses deleted chunks / not idempotent | #356, #490, #167 | Delete by map prefix |
| Skylight 15 assumed in Nether/End | #206 | Dimension-aware defaults |
| Players leaked from disabled worlds | #163 | Filter at source |
| One bad marker kills the webapp | #202 | Validate markers server-side |

Drop-in caveat: replicating *visual* quirks for no seams (see 00 "Product goal") is different from replicating
*robustness* bugs. Fix every bug in this table; copy only output-affecting behaviour.

## Must preserve (users depend on it)

- Static-host layout: `maps/<id>/settings.json`, `tiles/0` `.prbm(.gz)`, lowres `tiles/1..n` PNG, `textures.json`;
  `gzip_static` + emptyTile fallback; served via nginx, S3/R2, GitHub Pages (#872, #392, #293).
- HOCON keys and meanings: `full-update-interval`, `render-thread-count`, `ip`/`port`, `save-hires-layer`,
  `ignore-missing-light-data`, `min-inhabited-time`, `write-markers-interval`, `map.conf` marker-sets.
- SQL schema read by `sql.php`; per-map storage configs.
- Commands: render/update/force-update/fix-edges/purge/freeze/cancel/pause; persisted, resumable render state.
- File-watch incremental updates; CLI that renders offline on another machine.
- BlueMapAPI v2 marker contract (#403: the v1→v2 break killed addons). Note #764: Valkyrien Skies mixes into
  BlueMap internals (`TileModel.position[]`) — that mod breaks under any rewrite.

## Opportunities beyond parity (cheap wins users ask for)

- Memory budget config key + RSS reporting in `/bluemap status` (#565, #271).
- CPU budget: tiles/s cap, low-priority threads, pause on TPS/MSPT/player count (#215, #176, #246).
- Lowres-only mode / skip hires below `min-inhabited-time` (#500, #288, #140) — the second already exists, the first doesn't.
- Packed/SQLite storage to kill file-count pain (#701, #217) — the format must still be servable by sql.php.
- Resumable force-render (#695); viewer-only mode with no world (#660).
- Cache-Control/ETag on tiles (#17); optional TLS via rustls (#266, #132).
- Persisted resource cache keyed by jar hash for heavily modded startup (#73, #129).

## Out of scope for the backend

Most webapp-ui (85) and live-player UI issues, GPU/isometric/day-night features, and modded blocks that need
server-side renderers (Create, Chisels & Bits, CTM) are either client-side or impossible from JSON models alone.
