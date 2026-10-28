# Changelog

Versions follow `<crate version>`; the plugin and mod jars report `<BlueMap version>+rs.<crate version>`, the
BlueMap version they replace.

## Unreleased

- `optimized` storage, serving: browsers without zstd as a content coding (Safari before 26.3, Safari 26 on macOS
  before Tahoe, clients behind proxies that strip it) now get the stored zstd frame too, and the client script
  decodes it in WASM (zstd's C decoder, +39 KB to the module, +20 KB to the script on the wire). Server CPU per
  hires tile for them falls from 0.24–0.39 ms to 0.02–0.03 ms; tiles/s at 32 connections from 32k to 299k
  (docs/18). Pages still running the 0.3.0 script keep the gzip path.

## 0.3.0 — 2026-10-28

- Drop-in target is now BlueMap 5.29: its webapp, `core.conf` template (`region-file-check-interval`) and
  BlueMapAPI 2.8.2; jars report `5.29+rs.<crate version>`. Hires and lowres output is unchanged (5.29 renders
  the same tiles as 5.28).
- Webserver: error responses carry 5.29's `<code> - <message>` body; `Accept-Encoding` honours `*` and treats an
  unparsable `q` as 0, like 5.29.
- `optimized` storage, serving: hires tiles go out packed and the browser unpacks them (a WASM script added to
  `index.html`, negotiated by `Accept`; docs/18), so the server spends 0.02 ms of CPU per tile instead of rebuilding
  PRBM. Clients without the script still get PRBM; new `webserver.conf` setting `server-unpack`
  (`limited` default, `never`, `always`) bounds that work, so they can't use all the cores. `index.html` now
  revalidates, so it always loads the current script.
- `optimized` storage: a sidecar index per bundle, so a reader's first touch reads 2 KB instead of scanning the
  bundle; finest-first shape discovery makes hires data 3% smaller (format unchanged).
- Memory stays flat as the map grows: lowres tiles spill to storage past a 128 MB budget and render-state cells
  leave memory once no remaining region needs them (docs/22).
- Faster renders and updates: chunk parsing, palette reads, region headers read once per update, zlib through
  libdeflate, encoder and mesher hot paths (docs/20, 21, 23).
- Plugins: under a container memory limit (Pterodactyl and other panels) the core's `memory-limit` is set from the
  limit left after the JVM heap, and the decision is logged; a `memory-limit` in `core.conf` still wins.
- Docker image `ghcr.io/hexay/bluemap-rs` for amd64 and arm64, a drop-in for `ghcr.io/bluemap-minecraft/bluemap`:
  same mounts, arguments and port. `--help` honours `BLUEMAP_COMMAND` like upstream.
- musl cores use their own `memset` (zig's is a byte loop); glibc builds return freed memory when the render queue
  runs dry.
- Fixes: ARM and macOS builds (lanes.rs re-export, memfns transmute); dependency updates for RustSec advisories.

## 0.2.0 — 2026-10-09

- `optimized` storage: hires tiles are stored as BMQ3, which models how the renderer builds a tile. Hires data is
  about 0.4× the size of 0.1.0's, and tiles encode faster. An optimized storage written by 0.1.0 is not readable:
  render it again, or convert it to `compat` with 0.1.0 first and back with this version.
- World reads: an area's chunks are read from each region file up front, instead of chunk by chunk from every
  render thread on one file handle (which Windows serialises).

## 0.1.0 — 2026-10-09

First release: a drop-in replacement for BlueMap 5.28.

- `bluemap` CLI with BlueMapCLI's flags, exit codes and log format; renders byte-identical to Java BlueMap's hires
  output, from every chunk format since Minecraft 1.13.
- File and SQL storage (SQLite, MySQL/MariaDB, PostgreSQL) in BlueMap's layout; opt-in `optimized` storage format and
  `--convert-storage` between the two.
- Paper plugin and Fabric mod (dedicated servers, 26.1–26.3) running the core as a child process, with BlueMapAPI 2.8
  for marker plugins and addons.
- Beyond BlueMap: render pausing on server lag, `memory-limit`, low-priority render threads, resumable forced renders
  (`--restart` to start over). See `docs/15-beyond-parity.md`.
