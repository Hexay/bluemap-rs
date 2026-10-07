# bluemap-rs

A Rust reimplementation of [BlueMap](https://github.com/BlueMap-Minecraft/BlueMap), the 3D web map for Minecraft,
built as a drop-in replacement: keep your configs, rendered maps, web setup, commands and marker plugins, and get
lower CPU and memory use.

**Status:** early. The foundation (compression codecs, tile formats, a golden-test harness against Java BlueMap) is in
place; the world reader, renderer, storage and web server are not yet written. See `docs/00-overview.md` for the
design and plan.

## Layout

| Path | Contents |
|---|---|
| `crates/` | Rust workspace (`bm-*` crates) |
| `docs/` | Design overview and research on BlueMap's internals |
| `tools/` | Python harness: builds test worlds, renders them with Java BlueMap, benchmarks |
| `fixtures/` | Test world specs |

## Development

Rust 1.94+, Python 3.11+ (stdlib only) for `tools/`. See `CLAUDE.md` for commands.

## License

MIT. Ports formats and behaviour of BlueMap (MIT, © Blue). Not affiliated with the BlueMap project.
