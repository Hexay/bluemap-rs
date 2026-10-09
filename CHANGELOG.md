# Changelog

Versions follow `<crate version>`; the plugin and mod jars report `5.28+rs.<crate version>`, the BlueMap version
they replace.

## Unreleased

- `optimized` storage: hires tiles are stored as BMQ3, which models how the renderer builds a tile. Hires data is
  about 0.4× the size of 0.1.0's, and tiles encode faster. An optimized storage written by 0.1.0 is not readable:
  render it again, or convert it to `compat` with 0.1.0 first and back with this version.

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
