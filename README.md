# bluemap-rs

[![CI](https://github.com/Hexay/bluemap-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Hexay/bluemap-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A Rust reimplementation of [BlueMap](https://github.com/BlueMap-Minecraft/BlueMap), the 3D web map for Minecraft,
built as a **drop-in replacement for BlueMap 5.29**. Keep your configs, rendered maps, web setup, commands and marker
plugins; get the same map for a fraction of the CPU and memory.

> **Status: beta.** Not affiliated with the BlueMap project.

## Why

On a 4096×4096-block world (100 region files), a full render from scratch, same config, 12 render threads:

| | Wall time | CPU time | Peak memory | Output |
|---|---:|---:|---:|---:|
| Java BlueMap 5.28 | 5.7 min | 75 min | 2,899 MB | 1,545 MiB |
| bluemap-rs | **14.8 s** | **2.7 min** | **858 MB** | 1,576 MiB |
| bluemap-rs, `format: optimized` storage | **12.4 s** | **2.3 min** | 909 MB | **130 MiB** |

- **23× faster, 27× less CPU, 30% of the memory** (medians of 3 runs). Re-rendering after edits to 6 region files:
  0.97 s and 3.4 CPU-seconds, against Java's 4.3 s and 45 CPU-seconds.
- **Cheaper to serve.** For a hires tile nobody requested recently, the webserver uses 5–20× less CPU than Java's on
  the same compat map, 13–32× less with `optimized` storage, and under half its memory. A page reload revalidates
  instead of downloading the view again: 13 KB against 5.8 MB ([docs/results](docs/results/devbox-web-java-vs-rs-2026-10-28.md)).
- **The same map.** Hires tiles are byte-identical to Java's after decompression; render state and JSON are identical.
  The one lowres difference on that world is a hole in Java's output that bluemap-rs doesn't have
  ([docs/14](docs/14-real-world-validation.md)).
- **Light on a game server.** The Paper/Fabric core runs as a separate ~50 MB process instead of inside the server's
  JVM heap. It can pause while the server lags, cap its own memory, and resume interrupted renders
  ([docs/15](docs/15-beyond-parity.md)).

Measured on 2026-10-28 on an 8-core/16-thread Ryzen 7 9700X with 62 GB RAM and NVMe storage, shared with other jobs.
Both renderers ran on that machine, with the same world and config; bluemap-rs is the glibc build. The shipped static
Linux build uses 9% more CPU and 17% less memory. Method, raw numbers and caveats:
[docs/results/devbox-2026-10-28.md](docs/results/devbox-2026-10-28.md).

### Which storage format

Same world and machine; the better value in each row is bold:

| | `compat` (BlueMap's layout) | `optimized` |
|---|---:|---:|
| Render, wall time | 14.8 s | **12.4 s** |
| Render, CPU time | 2.7 min | **2.3 min** |
| Size on disk | 1,576 MiB in 16,865 files | **130 MiB in 387 files** |
| Serve a hires tile (server CPU) | 0.03 ms | **0.02 ms** ¹ |
| Bytes sent per hires tile | 94 KB | **7 KB** ¹ |
| Hires tiles served per second | 153,000 | **381,000** ¹ |
| Can be served by | **bluemap-rs, Java BlueMap, nginx, `sql.php`** | bluemap-rs only |

¹ With bluemap-rs's webserver, which has the browser unpack optimized tiles itself
([docs/18](docs/18-client-unpack.md)). A client that doesn't (another webapp, or `index.html` served by nginx) is
sent ordinary tiles, which the server has to rebuild; by default only a quarter of the cores do that at once,
the rest queue briefly, shared fairly per client (`server-unpack` in `webserver.conf`; behind a reverse proxy see
[docs/reverse-proxy.md](docs/reverse-proxy.md)).

Serving a hires tile nobody requested recently, by server and client:

| Server, storage, client | Server CPU per tile, 1 / 32 connections | Tiles per second, 32 connections | 512 connections | Sent per tile |
|---|---:|---:|---:|---:|
| Java BlueMap 5.28, `compat` | 0.65 / 0.32 ms | 1,016 ² | 54,060 | 94 KB |
| bluemap-rs, `compat` | 0.033 / 0.067 ms | 152,678 | 80,210 | 94 KB |
| bluemap-rs, `optimized`, browser unpacks | **0.020 / 0.025 ms** | **380,754** | **300,614** | **7 KB** |
| bluemap-rs, `optimized`, client without the script ³ | 1.8 / 2.0 ms | 2,103 | – | 97 KB |

² Java's server waits ~41 ms before each response after the first on a keep-alive connection (it never sets
`TCP_NODELAY`), so up to 32 connections its rate is connections ÷ 41 ms, not its CPU. The stall mostly goes away
under sustained load at 512 connections, where the client on the same machine limits both servers.

³ Each tile requested once (the server caches tiles it rebuilds); at most a quarter of the cores unpack at once.

Medians of 3 runs, 2026-10-28, page cache warm, none of the measured tiles requested earlier in the server's life.
With the page cache dropped first, so every tile comes from disk, 32 connections get 969 tiles per second from Java, 72,714 from
bluemap-rs `compat` and 187,268 from `optimized`, at 0.19 / 0.079 / 0.029 ms CPU per tile
([method and all rows](docs/results/devbox-web-java-vs-rs-2026-10-28.md)). The first request to a part of the map
nobody has viewed since startup reads one 2 KB index per storage bundle: 10,000–66,000 such reads per second at 32
connections, p99 under 9 ms.

## What works

| | |
|---|---|
| **CLI** (`bluemap`) | BlueMapCLI's flags, exit codes and log format: `-r`, `-u`, `-w`, `-f`, `-e`, `-g`, `-s`, `-m` |
| **Paper plugin** | Verified on Paper 26.3. Runs the core as a child process and ships the real BlueMapAPI 2.8, so marker plugins and addons keep working |
| **Fabric mod** | Dedicated servers, Fabric 26.1–26.3 (verified on 26.3) |
| **Worlds** | Every chunk format from Minecraft 1.13 to 26.x; checked against Java BlueMap on 1.16.5–26.3 worlds |
| **Storage** | File and SQL (SQLite, MySQL/MariaDB, PostgreSQL), the same layout and schema as BlueMap. Opt-in `optimized` format, about 12× smaller |
| **Web** | BlueMap 5.29's webapp, unchanged, from the built-in webserver or your own nginx/Apache. For `optimized` storage the built-in webserver adds one script so browsers unpack tiles themselves |

Folia 26.2 passes the plugin tests (maps, commands, addon markers), but nothing has tested it with players online.
Not yet: Spigot, NeoForge/Forge, Fabric singleplayer, and BlueMap's Java native addons (resource packs from
addon jars still load). See [MIGRATING.md](MIGRATING.md) for the full list.

## Install

Downloads are on the [Releases](https://github.com/Hexay/bluemap-rs/releases) page.

- **CLI**: unpack `bluemap-rs-<version>-<platform>` and run `bluemap` where you ran `java -jar bluemap-cli.jar`, from
  the same working directory. Builds for Linux (x64, arm64, armv7; static), Windows x64 and macOS (x64, arm64).
- **Docker**: replace `ghcr.io/bluemap-minecraft/bluemap:<version>` with `ghcr.io/hexay/bluemap-rs:<version>`
  (amd64, arm64). Same mounts, arguments and port.
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
