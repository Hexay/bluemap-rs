# Changelog

Versions follow `<crate version>`; the plugin and mod jars report `5.28+rs.<crate version>`, the BlueMap version
they replace.

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
