# bluemap-rs

A Rust reimplementation of [BlueMap](https://github.com/BlueMap-Minecraft/BlueMap), the 3D web map for Minecraft,
built as a drop-in replacement: keep your configs, rendered maps, web setup, commands and marker plugins, and get
lower CPU and memory use. Drop-in target: **BlueMap 5.28** (webapp shipped unchanged).

**Status:** pre-release, no published builds yet.

- **CLI** (`bluemap`): BlueMapCLI's flags, exit codes and log format; `-r`, `-u`, `-w`, `-f`, `-e`, `-g`, `-s`.
  Renders are byte-identical to Java BlueMap on every golden fixture; one forced render of the `structures` fixture
  took 14.7 s / 345 MB against Java's 86.9 s / 1238 MB (`docs/12-perf-profile-rs.md`).
- **Paper plugin** (`platforms/paper`) and **Fabric mod** (`platforms/fabric`, dedicated servers): small Java shims
  that run the Rust core as a child process and proxy BlueMapAPI 2.8, so marker plugins and addons keep working.
  Paper 26.3 and Fabric 26.3 verified; Folia, Spigot, NeoForge and singleplayer not yet.
- Worlds: all chunk formats from 1.13 to 26.x. Storage: file and SQL (SQLite, MySQL/MariaDB, PostgreSQL).

Switching from Java BlueMap: see [MIGRATING.md](MIGRATING.md).

## Layout

| Path | Contents |
|---|---|
| `crates/` | Rust workspace (`bm-*` crates, listed in `CLAUDE.md`) |
| `platforms/` | Java shims around the core: `common` (loader-neutral), `paper`, `fabric` |
| `docs/` | Design (`00-overview.md` first) and research on BlueMap's internals |
| `tools/` | Python harness: golden renders with Java BlueMap, acceptance, benchmarks, Paper/Fabric e2e |
| `fixtures/` | Test world specs |

## Development

Rust 1.94+, Python 3.11+ (stdlib only) for `tools/`, JDK 25 for the plugin/mod build. See `CLAUDE.md` for commands.

## License

MIT. Ports formats and behaviour of BlueMap (MIT, © Blue). Not affiliated with the BlueMap project.
