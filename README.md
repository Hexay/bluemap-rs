# bluemap-rs

[![CI](https://github.com/Hexay/bluemap-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Hexay/bluemap-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A Rust reimplementation of [BlueMap](https://github.com/BlueMap-Minecraft/BlueMap), the 3D web map for Minecraft,
built as a **drop-in replacement for BlueMap 5.28**. Keep your configs, rendered maps, web setup, commands and marker
plugins; get the same map for a fraction of the CPU and memory.

> **Status: beta, pre-release.** No published builds yet. Not affiliated with the BlueMap project.

## Why

On a 4096×4096-block world (100 region files), a full render from scratch, same config, 12 render threads:

| | Wall time | CPU time | Peak memory | Output |
|---|---:|---:|---:|---:|
| Java BlueMap 5.28 | 23.5 min | 2.7 h | 2,139 MB | 1,545 MiB |
| bluemap-rs | **77 s** | **7.9 min** | **764 MB** | 1,576 MiB |
| bluemap-rs, `format: optimized` storage | **49 s** | 8.3 min | 802 MB | **310 MiB** |

- **18× faster, 21× less CPU, 36% of the memory** (medians of 3 runs). Re-rendering after edits to 6 region files:
  5.0 s and 5.6 CPU-seconds, against Java's 20.5 s and 155 CPU-seconds.
- **The same map.** Hires tiles are byte-identical to Java's after decompression; render state and JSON are identical.
  The one lowres difference on that world is a hole in Java's output that bluemap-rs doesn't have
  ([docs/14](docs/14-real-world-validation.md)).
- **Light on a game server.** The Paper/Fabric core runs as a separate ~50 MB process instead of inside the server's
  JVM heap. It can pause while the server lags, cap its own memory, and resume interrupted renders
  ([docs/15](docs/15-beyond-parity.md)).

Measured on a 6-core/12-thread Xeon E-2136 with 31 GiB RAM and HDD storage. Method, raw numbers and caveats:
[docs/14](docs/14-real-world-validation.md).

## What works

| | |
|---|---|
| **CLI** (`bluemap`) | BlueMapCLI's flags, exit codes and log format: `-r`, `-u`, `-w`, `-f`, `-e`, `-g`, `-s`, `-m` |
| **Paper plugin** | Verified on Paper 26.3. Runs the core as a child process and ships the real BlueMapAPI 2.8, so marker plugins and addons keep working |
| **Fabric mod** | Dedicated servers, Fabric 26.1–26.3 (verified on 26.3) |
| **Worlds** | Every chunk format from Minecraft 1.13 to 26.x; checked against Java BlueMap on 1.16.5–26.3 worlds |
| **Storage** | File and SQL (SQLite, MySQL/MariaDB, PostgreSQL), the same layout and schema as BlueMap. Opt-in `optimized` format, about 5× smaller |
| **Web** | BlueMap 5.28's webapp, unchanged, from the built-in webserver or your own nginx/Apache |

Not yet: Folia, Spigot, NeoForge/Forge, Fabric singleplayer, and BlueMap's Java native addons (resource packs from
addon jars still load). See [MIGRATING.md](MIGRATING.md) for the full list.

## Install

Downloads will be on the [Releases](https://github.com/Hexay/bluemap-rs/releases) page.

- **CLI**: unpack `bluemap-rs-<version>-<platform>` and run `bluemap` where you ran `java -jar bluemap-cli.jar`, from
  the same working directory. Builds for Linux (x64, arm64, armv7; static), Windows x64 and macOS (x64, arm64).
- **Paper**: replace the BlueMap jar in `plugins/` with `bluemap-rs-paper-…-<platform>.jar` (or the `universal` jar).
  It keeps using `plugins/BlueMap/`. Java 21+.
- **Fabric**: replace the BlueMap jar in `mods/` with `bluemap-rs-fabric-…jar`. Needs Fabric API. Java 25.

The first start behaves like BlueMap's: it writes default configs and asks you to accept the Minecraft download in
`core.conf`. **Already running BlueMap?** Nothing to convert. Read [MIGRATING.md](MIGRATING.md) for what carries
over, what differs, and how to switch back.

## Build from source

```sh
cargo build --release -p bm-cli        # target/release/bluemap
py -3 tools/build_core.py linux-x64 --jars   # core for a target + Paper/Fabric jars (JDK 25); --list for targets
```

Rust 1.94+ and a C compiler (zstd and libdeflate are C). The static Linux cores cross-build with
`pip install ziglang cargo-zigbuild`.

## Documentation

- [MIGRATING.md](MIGRATING.md): switching from Java BlueMap, differences, switching back
- [docs/00-overview.md](docs/00-overview.md): design, decisions and an index of the research docs
- [CONTRIBUTING.md](CONTRIBUTING.md): development setup, golden tests against Java BlueMap
- [SECURITY.md](SECURITY.md): reporting vulnerabilities

## License

MIT. bluemap-rs ports the formats and behaviour of BlueMap (MIT, © Blue) and embeds its webapp. "BlueMap" is Blue's
project name; bluemap-rs is an independent reimplementation and is not affiliated with or endorsed by it.
