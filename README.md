# bluemap-rs

[![CI](https://github.com/Hexay/bluemap-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Hexay/bluemap-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A Rust rewrite of [BlueMap](https://github.com/BlueMap-Minecraft/BlueMap), the 3D web map for Minecraft. It's a
drop-in replacement for BlueMap 5.29: keep your configs, rendered maps, web setup, commands and marker plugins.

Beta. Not affiliated with the BlueMap project.

## How it compares to BlueMap

- **Rendering** is when your server turns the world into map files. This is where bluemap-rs is much faster (23×).
- **Viewing** is when someone opens the map in a browser. This is only modestly faster (1.3–3.6×, with `optimized`
  storage), because much of the time is the visitor's connection and BlueMap's own webapp, which bluemap-rs reuses.

### Rendering

A 4096×4096 world, 12 render threads:

| | BlueMap 5.28 | bluemap-rs `compat` | bluemap-rs `optimized` |
|---|---:|---:|---:|
| Full render | 5.7 min | 14.8 s | 12.4 s |
| CPU time | 75 min | 2.7 min | 2.3 min |
| Peak memory | 2,899 MB | 858 MB | 909 MB |
| Re-render after editing 6 region files | 4.3 s | 0.97 s | 0.89 s |
| Size on disk | 1,545 MiB | 1,576 MiB | 130 MiB |

### Viewing: the server

Hires tiles nobody requested recently, on the same machine as the test client:

| | BlueMap 5.28 | bluemap-rs `compat` | bluemap-rs `optimized` |
|---|---:|---:|---:|
| Server CPU per tile, 1 connection | 0.65 ms | 0.033 ms | 0.020 ms |
| Server CPU per tile, 32 connections | 0.32 ms | 0.067 ms | 0.025 ms |
| Tiles per second, 32 connections | 1,016 ¹ | 152,678 | 380,754 |
| Tiles per second, 512 connections | 54,060 | 80,210 | 300,614 |
| Data sent per tile | 94 KB | 94 KB | 7 KB |

¹ BlueMap's server pauses ~41 ms before most responses on a reused connection, which limits it here more than its CPU
does. Compare the 512-connection row for its capacity. At that load the test client limits BlueMap and `compat`.

Throughput is how many visitors a server can handle, not how fast the map loads for each one. Most servers never get
near these rates.

### Viewing: the browser

The real webapp in Chrome, on a 50 Mbit/s and a 10 Mbit/s connection:

| | BlueMap 5.28 | bluemap-rs `compat` | bluemap-rs `optimized` |
|---|---:|---:|---:|
| First load | 2.2 s, 5.1 s | 1.9 s, 4.4 s | 1.7 s, 2.9 s |
| Reload | 3.6 s, 10.8 s | 1.8 s, 3.2 s | 1.8 s, 3.2 s |
| New tiles appearing while panning | 197 ms, 429 ms | 204 ms, 433 ms | 95 ms, 119 ms |
| Frame rate while panning or zooming | same | same | same |

- **It isn't smoother.** The webapp and tile geometry are the same as BlueMap's, so frame rate matches. Freezes during
  loading are about the same length, and a reload may have a few more slow frames. The map fills in sooner.
- **Reloads are faster on both storages** because they don't download everything again.
- **The cost:** clients that can't unpack `optimized` tiles are slower to serve than with BlueMap (see below).

Hires tiles are byte-identical to BlueMap's after decompression; render state and JSON are identical too. The one
lowres difference on the test world is a hole in BlueMap's output that bluemap-rs doesn't have.

Tested on a Ryzen 7 9700X with NVMe storage, using a glibc build; the static Linux release uses 9% more CPU and 17%
less memory. The browser tests used real Chrome on an Intel Arc GPU, on one world, and haven't covered phones or
discrete GPUs. Methods and raw numbers:
[rendering](docs/results/devbox-2026-10-28.md), [serving](docs/results/devbox-web-java-vs-rs-2026-10-28.md),
[browser](docs/results/webapp-smoothness-2026-11-01.md).

## Which storage format

`compat` is the default and BlueMap's own layout. Any BlueMap setup can serve it: bluemap-rs, Java BlueMap, nginx or
`sql.php`.

`optimized` is opt-in and works only with bluemap-rs's webserver. Tiles are stored and sent packed, and a small script
(about 45 KB, cached) unpacks them in the browser.

Clients without that script, such as an `index.html` served by nginx, still work. The server has to rebuild each tile
for them, at about 2 ms of CPU per tile, 3–6× BlueMap's cost. bluemap-rs limits that work so it can't take over the
server ([docs/18](docs/18-client-unpack.md), [reverse proxies](docs/reverse-proxy.md)).

Use `optimized` if bluemap-rs's webserver serves your map. Use `compat` if you serve tiles another way.

## What works

- **CLI:** BlueMapCLI's flags, exit codes and log format.
- **Paper:** verified on 26.3. BlueMapAPI 2.8 is included, so marker plugins and addons keep working.
- **Fabric:** dedicated servers on 26.1–26.3 (verified on 26.3).
- **Worlds:** Minecraft 1.13 to 26.x, checked against BlueMap on 1.16.5–26.3.
- **Storage:** files and SQL (SQLite, MySQL/MariaDB, PostgreSQL).
- **Web:** BlueMap 5.29's webapp, from the built-in webserver or your own nginx/Apache.

Not yet: Spigot, NeoForge/Forge, Fabric singleplayer, and BlueMap's Java native addons. Folia passes the plugin tests
but hasn't been tested with players online. [MIGRATING.md](MIGRATING.md) has the full list.

## Install

Download from [Releases](https://github.com/Hexay/bluemap-rs/releases).

- **CLI:** unpack it and run `bluemap` where you ran `java -jar bluemap-cli.jar`.
- **Docker:** replace `ghcr.io/bluemap-minecraft/bluemap:<version>` with `ghcr.io/hexay/bluemap-rs:<version>`.
- **Paper:** replace the BlueMap jar in `plugins/`. Java 21+.
- **Fabric:** replace the BlueMap jar in `mods/`. Needs Fabric API and Java 25.

Already running BlueMap? There's nothing to convert. [MIGRATING.md](MIGRATING.md) covers what carries over, what
differs and how to switch back.

## Build from source

```sh
cargo build --release -p bm-cli              # target/release/bluemap
py -3 tools/build_core.py linux-x64 --jars   # core for a target + Paper/Fabric jars (JDK 25); --list for targets
```

Needs Rust 1.94+ and a C compiler. The static Linux cores cross-build with `pip install ziglang cargo-zigbuild`.

## More

- [MIGRATING.md](MIGRATING.md): switching from BlueMap and back
- [docs/00-overview.md](docs/00-overview.md): design and decisions
- [CONTRIBUTING.md](CONTRIBUTING.md): development and tests
- [SECURITY.md](SECURITY.md): reporting vulnerabilities

## License

MIT. bluemap-rs ports the formats and behaviour of BlueMap (MIT, © Blue) and embeds its webapp. "BlueMap" is Blue's
project name; bluemap-rs is independent and not endorsed by it.
