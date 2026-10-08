# Switching from Java BlueMap

bluemap-rs reads your existing BlueMap 5.28 setup as-is. You don't re-render, rewrite configs or touch your web
setup.

## What carries over unchanged

| You have | It keeps working |
|---|---|
| `core.conf`, `webserver.conf`, `webapp.conf`, `plugin.conf`, `maps/*.conf`, `storages/*.conf` | Same keys and defaults |
| Rendered maps (file or SQL storage) | Served as-is and updated incrementally (render state in `rstate/` is read and written) |
| nginx/apache in front of the file tree, `sql.php` | Same paths, file suffixes and SQL schema, as long as the storage stays `compat` (below) |
| A customised `web/` folder | Left alone. The same webapp version is embedded and used only for files you don't have |
| CLI scripts and systemd units | Same flags and exit codes (1: config error, 2: missing resources) |
| Marker plugins (BlueMapAPI) and `depend: BlueMap` | The plugin is named `BlueMap` (Fabric: mod id `bluemap`, version `5.28+rs.…`) and ships the real API classes |
| `/bluemap …` commands and permissions | Same command tree and permission nodes |

You can switch back to Java BlueMap later; see "Switching back" below.

## CLI / Docker

Replace `java -jar bluemap-cli.jar …` with `bluemap …`. Run it from the same working directory, because BlueMap
config paths are relative to the working directory. `bluemap --version` prints the BlueMap version it is a drop-in
for, then its own version.

## Paper plugin

1. Stop the server and remove the BlueMap jar from `plugins/`. bluemap-rs refuses to start next to it, because both
   would render the same maps.
2. Put the bluemap-rs jar in `plugins/` and start the server. It keeps using `plugins/BlueMap/`.
3. On start, the plugin extracts its core binary to `plugins/BlueMap/bin/<platform>/` and runs it as a child
   process. Supported platforms: `linux-x64`, `linux-arm64`, `windows-x64`, `macos-x64` and `macos-arm64`.
   On any other platform, or where the server folder can't execute files, build or download the core yourself and
   point the plugin at it with `-DBLUEMAP_CORE=/path/to/bluemap` or the `BLUEMAP_CORE` environment variable.

Requires Java 21 or newer. Built against the Paper 1.21.11 API and verified on Paper 26.3; older 1.21.x servers
should work but are untested. Folia is wired up but not yet tested.

Unlike upstream, rendering pauses while the server lags: when the average tick time over 10 s goes above 45 ms, and
resumes below 40 ms. Change this with `render-pause-mspt` / `render-resume-mspt` in `plugin.conf` (`0` turns it off).
Render threads also run at low OS priority. On Folia only the low priority applies.

## Fabric mod

1. Stop the server and remove upstream's BlueMap jar from `mods/`. Both are mod `bluemap`, so Fabric would load only
   one of them, and bluemap-rs stays inactive if it sees upstream's jar.
2. Put the bluemap-rs Fabric jar in `mods/` (Fabric API required, as upstream). Config stays in `config/bluemap/`,
   the core binary goes to `config/bluemap/bin/<platform>/`.

Dedicated servers only, Minecraft 26.1–26.3, Java 25. In singleplayer or on a LAN world the mod does nothing (it logs
one line), so a modpack can keep it on the client side too. Fabric addons that `depend` on `bluemap` keep working.
Permission nodes go through fabric-permissions-api (LuckPerms etc.); without one, `/bluemap` needs operator status (the "moderators" level, as upstream).
Rendering pauses on server lag as on Paper (`render-pause-mspt` / `render-resume-mspt` in `plugin.conf`).

## Memory and thread limits (Pterodactyl, Pelican and other panels)

The core runs outside the Java heap, so the container's memory limit now covers **the JVM plus the core**. The
stock Paper egg starts Java with `-XX:MaxRAMPercentage=95.0`, which leaves the core almost nothing; the host may
then kill the whole server. Lower the percentage so the core keeps some headroom. How much it needs grows with
render threads and map size.

To cap the core itself, add `memory-limit: "1G"` (any size like `512M`, `2GiB`) to `core.conf`. The core then
starts fewer render threads to fit, and pauses rendering while it is above the limit (the map keeps being served).
See docs/15-beyond-parity.md.

Panels also cap the number of threads per container (default 512 on Pterodactyl), and the JVM and the core share
that cap. If you hit the cap, lower `render-thread-count` in `core.conf`.

## Storage format

Storage configs created by bluemap-rs get a new setting:

```hocon
format: optimized
```

- **Existing storages** (no `format` setting, or written by Java BlueMap) stay `compat`, BlueMap's original
  layout. Nothing changes on disk.
- **`optimized`** packs hires tiles into a compact encoding that is several times smaller and faster to write. The
  webapp is unchanged, but only bluemap-rs's own webserver can serve it. nginx on the file tree and `sql.php`
  need `compat`.
- A storage records its own format, and a config that disagrees with it is refused rather than guessed.
  Converting is a separate step. Stop BlueMap first, then:

```sh
bluemap --convert-storage <storage-id> --to optimized   # or: --to compat
```

On a server, run the extracted core from the server folder with the plugin's config folder:
`plugins/BlueMap/bin/<platform>/bluemap-core-<version> -c plugins/BlueMap --convert-storage <storage-id> --to optimized`.
If the storage config can't be rewritten automatically, the command tells you which `format` line to set.

## Not supported

- **Java addons** (`packs/*.jar` containing `bluemap.addon.json`, e.g. BlueMapBrotli, BlueMap-Linear, S3Storage,
  Entities) can't run on a Rust core. Each one is logged by name at startup and skipped. Any resource packs inside
  the jar still load.
- **`BlueMapMap.setTileFilter`** (deprecated upstream) is ignored, with a warning.
- **Casting the API to its internals** (`BlueMapAPIImpl`, `BlueMapMapImpl`, core classes) throws
  `UnsupportedOperationException`. Plugins that only use the public API are unaffected.
- Command output matches upstream's wording and colours but isn't component-identical. The core also logs one line
  per finished render task.
- Metrics never go to BlueMap's bStats or metrics endpoint. bluemap-rs reports to its own bStats id, and respects
  `metrics: false`.

## Switching back

The render state and `textures.json` stay in upstream's formats. To return to Java BlueMap:

1. Convert every `optimized` storage back with `--to compat`.
2. Remove the `format` lines (Java BlueMap ignores them anyway).
3. Swap the jar or binary back.
