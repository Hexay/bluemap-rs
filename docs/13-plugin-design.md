# 13 — Server plugin: Java shim + Rust core over IPC

Scope: the Paper plugin (Fabric: docs/16; Forge/NeoForge later, same core). Settled in 00 "Decisions": separate process, one
jar with per-OS binaries, Java addons warn-only, drop-in for configs/commands/permissions/BlueMapAPI consumers.
Upstream refs: BlueMap `v5.28` (`0f3a9fb`; most refs were read at master `84ee993`), BlueMapAPI `bd7a9de` (2.8.x). Prefixes as in 05: `C/` common,
`CORE/` core, `IMPL/paper/` = `implementations/paper/src/main/java/de/bluecolored/bluemap/bukkit/`, `API/`.

## 1. What the server side does today, and who owns it after the split

Upstream Paper glue is 6 files / ~740 lines (`IMPL/paper/*`). Everything else is `C/plugin/Plugin.java`, which the
core already reimplements for the CLI. Rule: **JVM-only facts and callbacks stay in Java; all state, files, config,
rendering, web and command logic live in the core.**

| Upstream behaviour (evidence) | Owner | IPC |
|---|---|---|
| Save all worlds once at enable so `level.dat` exists, skipped on Folia (`BukkitPlugin.java:102-108`) | Shim, before spawn | — |
| MC version (`ServerBuildInfo…minecraftVersionId`, `:166-169`), config folder = plugin data folder (`:219`), mods folder `mods` (`:224`) | Shim | `Hello` |
| World list: Paper uses **one** `getLevelDirectory()` + dimension key + env→dimension type (`BukkitWorld.java:48-60`); only consumed for auto-generated map configs and API lookups (`Plugin.java:150`, `BlueMapAPIImpl.getWorldUncached`) | Shim reports, core decides | `Hello.worlds`, `WorldAdded/Removed` (Multiverse) |
| Default-blockstate registry dump → `data/defaultBlockstates.zip` (`BukkitPlugin.java:260-278`, `C/BlueMapService.java:435-473`) | Shim dumps (`Material` registry), core writes the zip (it owns `data`) | `Hello` body |
| Player snapshot 1 Hz on the entity scheduler, 20 ticks (`BukkitPlugin.java:286-290`; comment says "20 seconds", code is 20 ticks): name, world, pos, pitch/yaw, sky/block light at feet, sneaking, invisibility potion, `vanished` metadata, gamemode (`BukkitPlayer.java:134-158`) | Shim snapshots (server thread) | `Players` 1 Hz, one batch |
| Join/leave (`EventForwarder`, `BukkitPlugin.java:236-253`) → skin update + `player-render-limit` check 1 s later (`Plugin.java:694-733`) | Shim fires skins; core does render pause | `PlayerJoin/Leave` |
| Live-player filtering: hidden list, `hide-invisible/vanished/sneaking`, `hidden-game-modes`, light thresholds (`C/live/PluginLivePlayerInfoTransformer.java:53-77`), `hide-different-world`, JSON shape (`C/live/LivePlayersDataSupplier.java:56-102`) | **Core** (owns plugin.conf + pluginState) | display name precomputed by shim (`PlayerDisplayNameProvider` is a JVM callback) |
| Chunk/world-save events: **none**. Freshness is purely file-based (`C/plugin/MapUpdateService.java`, 05 §2); the only server hook is `/bluemap update` → `persistWorldChanges()` = `world.save()` on the main thread, false on Folia (`BukkitWorld.java:62-76`, `UpdateCommand.java:161`) | Core watches files; shim executes saves | `SaveWorld{world}` core→shim RPC |
| Skins: on join, ≤1/h per player, `SkinProvider` (Mojang default) → `PlayerIconFactory` → `assets/playerheads/<uuid>.png` in every map storage (`C/plugin/skins/PlayerSkinUpdater.java`) | **Shim** (both are replaceable JVM callbacks, e.g. floodgate) | `AssetWrite` per map |
| Commands: BlueCommands tree bridged to Brigadier in `LifecycleEvents.COMMANDS` (`BukkitPlugin.java:116-130`), executed on `BlueMap.THREAD_POOL` (`C/commands/CommandExecutor.java`) | Tree + permission gate in shim, execution in core | `Command` → `CommandOutput*` + `CommandDone` |
| Webserver, live JSON/SSE, timers (save 10 min, write-markers/players intervals, hourly watcher restart, full updates, tasks.dat), pluginState.json, metrics (`Plugin.java:213-419`) | Core | — |
| Marker sets (API POJOs) + config marker sets (`C/BlueMapService.java:241`) | Shim holds the live objects | `Markers` push (§3) |
| bStats (`new Metrics(this, 5912)`, `BukkitPlugin.java:152`) | Shim — **open question Q6** | — |
| Logging (`JavaLogger` over the plugin logger) | Core emits, shim prints | `Log{level,msg,trace}` |

### Commands (all under `/bluemap`, from `C/commands/commands/*.java`, mounted in `Commands.java:88-119`)

Permissions are not declared in `plugin.yml` (`IMPL/paper/.../plugin.yml`), so Bukkit defaults them to op.
"Ctx" = needs sender world/position (`@WithWorld`/`@WithPosition`). Every command runs on core state.

| Command | Permission | Ctx | Server call |
|---|---|---|---|
| *(none)* status | `bluemap.status` | | |
| `version` | `bluemap.version` | | shim adds shim + core version |
| `help` | `bluemap.help` | | |
| `reload`, `reload light` (`@Unloaded`) | `bluemap.reload`, `bluemap.reload.light` | | API disable/enable cycle (§3) |
| `start`, `stop` | `bluemap.start`, `bluemap.stop` | | |
| `maps` | `bluemap.maps` | | |
| `freeze <map>`, `unfreeze <map>` | `bluemap.freeze`, `bluemap.unfreeze` | | |
| `purge <map>` | `bluemap.purge` | | |
| `update\|fix-edges\|force-update` `[<map>] [<x> <z>] [<radius>]` (6 shapes) | `bluemap.update` (same node for all three) | world, pos for radius forms | `SaveWorld` before scheduling |
| `tasks`, `tasks cancel all\|<task-ref>` | `bluemap.tasks`, `bluemap.tasks.cancel` | | |
| `storages`, `storages <s>`, `storages <s> delete <map>` | `bluemap.storages`, `bluemap.storages.delete` | | |
| `troubleshoot [<map> [<x> <z>]]` | `bluemap.troubleshoot` | | |
| `debug dump` (`@Unloaded`), `debug world …`, `debug map …` | `bluemap.debug.dump/world/map` | world, pos | |

Design: the core owns a **command tree spec** (paths, argument kinds, permission, ctx flags) and exports it at build
time (`bluemap --dump-command-tree` → resource in the jar), so the shim can build the Brigadier tree in
`LifecycleEvents.COMMANDS` before the core is running (that event fires at startup). Argument suggestions (map,
storage, task-ref) come from the mirrored state or an async `Suggest` RPC (Brigadier suggestions are
`CompletableFuture`s). Execution sends `{path, args, sender:{kind, name, world, pos}}`. The core answers with
vanilla text-component JSON, which Paper (`GsonComponentSerializer`) and Fabric/NeoForge (`Component.Serializer`)
both parse. It can stream several messages, as `update` does ("saving…", then "scheduled").

## 2. BlueMapAPI surface and real-world usage

API classes (`API/`): `BlueMapAPI` (static `onEnable/onDisable/unregisterListener/getInstance`; `getMaps`, `getWorlds`,
`getWorld(Object)`, `getMap(id)`, `getWebApp`, `getRenderManager`, `getPlugin`, `getBlueMapVersion`, `getAPIVersion`),
`BlueMapWorld` (`getId`, `getSaveFolder`, `getMaps`), `BlueMapMap` (`getId`, `getName`, `getWorld`, `getAssetStorage`,
`getMarkerSets`, `getTileSize/Offset`, `posToTile`, `set/isFrozen`, deprecated `set/getTileFilter`), `AssetStorage`
(`writeAsset`→`OutputStream`, `readAsset`, `assetExists`, `getAssetUrl`, `deleteAsset`), `WebApp` (`getWebRoot`,
`set/getPlayerVisibility`, `registerScript/Style`, deprecated `createImage/availableImages`), `RenderManager`
(`scheduleMapUpdateTask` ×3, `scheduleMapPurgeTask`, `renderQueueSize`, `renderThreadCount`, `isRunning`,
`start`, `start(n)`, `stop`), `plugin.Plugin` (get/set `SkinProvider`, `PlayerIconFactory`,
`PlayerDisplayNameProvider`), markers (`MarkerSet` = `ConcurrentHashMap` POJO, no change events; `POI/HTML/Line/
Shape/Extrude` markers; `MarkerGson`), `math.Color/Shape/Line`. Impl unwraps: `BlueMapAPIImpl.blueMapService()/plugin()`,
`BlueMapMapImpl.map()` expose internals (`C/api/*.java`) and **cannot be supported**.

### Survey of real consumers

The survey covered 39 API users; 37 had source available and were grepped from shallow clones. Candidates came
from BlueMap's addon list (`BlueMapWiki/assets/addon_browser/addons.conf`) and the top Modrinth results. Download
counts are from Modrinth (MR) or SpigotMC.

| Addon | Size | Beyond baseline (`onEnable` + `getWorld/getMaps` + `getMarkerSets` + marker builders) |
|---|---|---|
| ChunkyBorder | 375k MR + 48k Spigot | Shape (ellipse) |
| Flan | 250k MR | Shape/Extrude |
| HuskHomes | 226k MR + 89k Spigot | POI, **writes `getWebRoot()/icons/huskhomes/`** |
| SimpleClaimSystem | 45k MR | Extrude |
| bmarker (MiraculixxT) | 40k MR | all marker types, `MarkerGson`, `setPlayerVisibility` |
| Create BlueMap / ComputerCartographer | 15k / 14k MR | Line/POI; **AssetStorage write/exists/url** |
| bbanner | 13.5k MR | `MarkerGson`, **`createImage`** |
| Offline Player Markers (+ BMUtils) | 18k MR | **AssetStorage, `getSkinProvider`, `getPlayerMarkerIconFactory`, webroot copy + `registerScript/Style`**, core `Logger` |
| Sign Markers, Frontiers, Towny, opac, Konquest, BlueBridge (WG/GP/Towny), HuskTowns/Claims, BlueBorder, vane, xclaim | 1–11k MR each | markers only (BlueBorder: `getWorld(String)`) |
| banners (RealMuffinTime), Banners4BM | 4k / 2k MR | AssetStorage, `MarkerGson` |
| MapTowny, TownyProvinces, Shopkeepers, minecolonies, KaiijuMC signs | ≤25★ | `createImage` / **direct webroot writes**, `registerScript/Style` |
| Floodgate, Custom Skin Provider | 20★ / 2★ | `setSkinProvider` only (Floodgate also core `Logger` via BMUtils) |
| Player Control | 6★ | `set/getPlayerVisibility` |
| Structures | 8★ | `BlueMapWorld.getSaveFolder()` (reads regions itself) |
| Area Control, MC Map Sync | 3★ each | **`setTileFilter`**, `posToTile`, `getTileSize/Offset`; Map Sync: **`getRenderManager().start/stop/isRunning`** |
| Sign Extractor, New Map | 16★ / 1★ | **internals**: `BlueMapMapImpl`, `MapUpdateService`, MCA classes, `BlueMapAPIImpl` cast + reflection |
| Lands, BlueMap-Essentials, Residence, GDHooks | closed/missing | unverified |

Frequency across 37: `getMarkerSets` ~33, `onEnable` ~33, `getMaps` ~31, `getWorld(Object)` ~30 (Bukkit `World`,
`ServerLevel`, UUID, name), `onDisable` ~18, POI ~18, Shape ~17, Extrude ~13, Html 4, Line 4, `getWebApp` ~11, `getWebRoot` ~10
(**every one writes files**), `registerScript/Style` ~7, AssetStorage write ~7 (read/delete 0), `MarkerGson` 6,
skin/icon providers 2–3, player visibility 2–3, `createImage` 2, `setTileFilter` 2, `getRenderManager` 1.
**Never used**: `scheduleMapUpdateTask/PurgeTask`, `setFrozen`, `renderQueueSize`, `availableImages`,
`PlayerDisplayNameProvider`, `setPlayerMarkerIconFactory`.

**Takeaway.** Two groups cover ~95% of real use:
- Lifecycle, world/map lookup and marker sets.
- `getWebRoot()` as a *writable, statically served local directory*, plus script/style registration, per-map
  AssetStorage writes and skin providers.

The tail is tile filter, render manager, `createImage`, `getSaveFolder` and the internals. Consequences:
- The webroot must stay a real local folder served from disk (§6).
- The shim also ships a tiny `de.bluecolored.bluemap.core.logger.Logger` facade (BMUtils → Floodgate, Offline
  Player Markers), Q5.
- The four native `bluemap.addon.json` addons (BlueMapBrotli, -Linear, S3Storage, Entities) touch core registries
  only, so they fall under warn-only. MapLink (413k MR) is an HTTP client and needs web byte-compat, not the API.

## 3. API proxy in the shim

The shim ships the real `de.bluecolored.bluemap.api` classes unrelocated (plugins compile against them), plus
`flow-math` through `plugin.yml libraries` as upstream does. It implements `BlueMapAPI` with three kinds of method:

| Kind | Methods | Mechanism |
|---|---|---|
| Mirror reads (never block on IPC) | `getMaps/getWorlds/getWorld/getMap`, map `getName/getTileSize/getTileOffset/isFrozen`, `getWebRoot`, `getPlayerVisibility`, `getAssetUrl`, `getBlueMapVersion` | from `Ready` (+ `StateChanged` pushes) |
| JVM-local objects | `getMarkerSets()` (shim-owned `ConcurrentHashMap` per map), `Plugin` providers, `createImage/availableImages` (plain file I/O under webroot, the same code as upstream) | none / periodic push |
| RPCs (sync request + timeout ~5 s) | `RenderManager.*`, `setFrozen`, `setPlayerVisibility`, `registerScript/Style`, `AssetStorage` write/read/exists/delete | request/response by id |

`getWorld(Object)`: resolves `World`/UUID/name/key in Bukkit (`BukkitPlugin.getServerWorld(Object)` `:191-212`), maps it
to (folder, dimension), then to the core's world id via the table in `Ready`.
`writeAsset` returns a buffering `OutputStream` that sends `AssetWrite` on `close()` (assets are icons, KB-sized).
With the core down, the RPCs throw `IOException`/return `false`, as the signatures allow.

**Markers: shim serializes, core serves.** Upstream never diffs. It runs `MarkerGson.toJson(map.getMarkerSets())` on demand, cached
10 s (`C/live/LiveMarkersDataSupplier.java:42`, `C/web/LiveDataSupplierBroadcaster` with 10000 ms in
`MapRequestHandler.java:91`), and writes the same JSON to storage on every map save and on `write-markers-interval`
(`CORE/map/BmMap.java:185,235-240`, `Plugin.java:326-335`). Markers are mutable POJOs with no events, so per-marker diffs
would need a deep compare anyway. Design:
- Every 10 s (one shim timer thread) each map's sets are serialized with the real `MarkerGson`, so the output is
  byte-identical by construction. If the string differs from the last push, send `Markers{map}` with the raw JSON
  as frame body.
- The core keeps the last JSON per map. It serves it at `live/markers.json` and sends the SSE `marker` event
  (bm-web `LiveMap::set_markers` already has this shape). It writes the JSON to storage on map save and on the
  interval. It validates the JSON before serving (#202, a bad marker kills the webapp).
- Config `marker-sets`: the core sends each map's config marker JSON in `Ready`. The shim
  `MarkerGson.fromJson`s it into the map's set before `onEnable` consumers run (upstream order,
  `C/BlueMapService.java:241`). The shim's map is then the single source and the core's own writer is CLI-only.
- Optional: skip serialization while the core reports no viewers and no pending save (`MarkerDemand`). This cuts
  idle cost for multi-MB claim maps. Upstream also only serialized on request.

**Lifecycle and threading** (`C/plugin/Plugin.java:129-571`, `API/BlueMapAPI.java:164-231`):
- `onEnable`: save worlds, register listeners and the command tree, then start a `BlueMap-Load` thread. That thread
  extracts the binary, spawns the core, sends `Hello` and waits for `Ready`. Then it seeds markers and calls
  `registerInstance`. The `onEnable` consumers run on that thread, async, which matches the upstream contract ("likely
  called asynchronously").
- `NotReady{missing-resources|config-error}`: the API stays unregistered. The core keeps the webserver up
  (`unload(true)` keeps it upstream, `:290-294`). `/bluemap reload` retries.
- `reload`: `unregisterInstance` (the `onDisable` consumers see a working API), then `Reload` to the core, then the
  new `Ready`. The shim builds a **new** API instance with fresh marker maps and calls `registerInstance`. Plugins
  re-add their markers in `onEnable`, as they do after an upstream reload.
- Server stop (`onDisable`, main thread): `unregisterInstance`, flush markers once, `Shutdown`, then wait for the
  core to exit (bounded, 30 s, then kill). The core does upstream `unload()`: tasks.dat, pluginState, map save.
- Core crash: **keep the API registered**, because the JVM-side state (markers, providers, scripts) survived.
  Respawn with backoff (1, 2, 4 … 60 s). After 5 crashes in 10 min, stop and tell the user to `/bluemap reload`.
  On reconnect, replay `Hello`, then everything since: all marker JSON, registered scripts/styles, the current
  player batch and frozen/visibility changes made while the core was down (the core persists the rest in
  pluginState). If `Ready` shows a different map set, which only happens when configs were edited, run a full
  reload cycle instead.
- `setTileFilter` (deprecated for removal, evaluated per tile in the render loop, `CORE/map/BmMap.java:141`):
  **Q4**. It could be served as one batched upcall per region task (tile list in, bitmask out), or dropped with a
  one-time warning.

## 4. IPC

| | stdin/stdout pipes | Unix socket (AF_UNIX, Win10 1803+) | Loopback TCP |
|---|---|---|---|
| Java side | `Process` streams, zero deps | `SocketChannel.open(UNIX)`, Java 16+ incl. Windows | trivial |
| Rust side | std | unix: tokio; Windows: tokio has no `UnixListener` → `uds_windows`/`interprocess` crate | tokio |
| Rendezvous | none | socket path (~108-byte limit, stale file cleanup, data folder may be long) | port handoff + auth token (other local users/containers can connect) |
| Parent-death signal | **free: EOF when the JVM dies, even on SIGKILL** (kernel closes fds) | none (need watchdog) | none |
| Reattach / external debug client | no | yes | yes |
| Hazard | stray `println!` corrupts framing | path perms | firewall/AV prompts, exposure |

**Recommend stdin/stdout.** It is the only option with zero rendezvous and it has a built-in liveness signal. Rules:
- In plugin mode the core first `dup`s fd 1 to a private handle and points fd 1 at stderr, so a stray print cannot
  corrupt the frame stream. Logs go as `Log` frames. stderr is read by a shim thread and forwarded line by line at
  WARN, which catches panics and C-library noise.
- Framing: `u32 len | u32 header_len | header (JSON) | body (raw bytes)`. Headers are small JSON objects
  `{t:"Markers", id, map, …}`. Bulk data travels as a raw body: marker JSON (no double escaping), PNGs, asset bytes
  and the blockstate dump. The Java side uses Gson, which Paper/MC already bundle, so nothing is shaded. Body cap
  64 MiB. Marker JSON above ~1 MiB is sent in 1 MiB continuation frames so player batches are not delayed
  (head-of-line on a single pipe).
- CBOR/MessagePack would add a shaded Java lib for no measurable gain at these rates (players 1 Hz, markers ≤0.1 Hz,
  logs). Protobuf adds codegen and is opaque in logs. JSON headers are greppable with `--ipc-trace`.
- Backpressure: one writer thread per side with a bounded queue. Player batches are latest-wins (drop stale).
  Markers are latest-wins per map. RPCs and logs are never dropped. When the log queue is full, lines are dropped
  and summarised as "N log lines dropped".
- Requests carry `id`; replies are `{t:"Reply", id, ok, err}`. Both directions issue requests (core→shim:
  `SaveWorld`, `Suggest`). Versioning: shim and core ship in one jar, so `Hello.protocol` must match exactly.
- Child cleanup: (1) stdin EOF → graceful stop (flush state) → exit; (2) `--parent-pid` watchdog in the core
  (`pidfd`/`kill(pid,0)` poll, `OpenProcess`+`WaitForSingleObject` on Windows) in case the pipe is inherited by
  grandchildren; (3) the core takes an exclusive lock on `<data>/.core.lock` and writes its pid. A new shim that finds
  the lock held kills that pid before spawning, because two cores rendering one map corrupt rstate.
  `PR_SET_PDEATHSIG` is a trap here: it fires when the *spawning thread* exits, and that is `BlueMap-Load`. Windows
  job objects need the parent to create the job, which Java 21 cannot do without FFM (preview). Both are skipped.
- Signals: in plugin mode the core ignores SIGINT and console Ctrl events (`SetConsoleCtrlHandler`). A Ctrl+C in the
  server terminal reaches the whole process group or console, and the core must stop only on the shim's
  `Shutdown`/EOF.

### Messages (≈25)

Shim→core: `Hello{protocol, platform, mcVersion, shimVersion, configFolder, modsFolder, metrics, folia, worlds[]}`
+ body (blockstates), `WorldAdded/Removed`, `Players{list}`, `PlayerJoin/Leave`, `Markers{map}`+body, `Command`,
`Suggest`, `Schedule{map, regions?, force}`, `Purge`, `RenderStart{threads?}`, `RenderStop`, `RenderStatus`,
`SetFrozen`, `SetPlayerVisibility`, `RegisterScript/Style`, `AssetWrite/Read/Exists/Delete`, `Reload{light}`,
`Shutdown`.
Core→shim: `Ready{coreVersion, compatVersion:"5.28", worlds[], maps[{id,name,world,tileSize,tileOffset,frozen,
configMarkers}], storages[], hiddenPlayers[], pluginConfig{skinDownload, …}, webroot}`, `NotReady{reason}`,
`StateChanged`, `Log{level, msg, trace}`, `CommandOutput{id, component}`, `CommandDone{id}`, `SaveWorld{id, world}`,
`MarkerDemand{maps}`, `Reply`.

## 5. Packaging and distribution

Measured here: `target/release/bluemap.exe` (x86_64-pc-windows-msvc, thin LTO, webapp embedded 1.7 MB) =
**12.9 MB**, 5.1 MB gzip -9, 3.9 MB xz -9. Guess for the others: ±20 % (Linux with stripped symbols is similar,
macOS slightly smaller). Six targets deflated in a jar come to ≈ **30 MB**, against 6.3 MB for upstream's
`bluemap-5.28-paper.jar`.

| Venue | Upload limit | Notes |
|---|---|---|
| Hangar | **10,000,000 B** (`/api/internal/data/validations` `maxFileSize`) | external URL per platform allowed but discouraged; bans obfuscation; silent on natives/exec |
| Modrinth | 500 MiB (labrinth `version_creation.rs:970`) | "Delphi" scanner flags `RUNTIME_EXEC_USAGE`/`NATIVE_LIBRARY_LOAD` → manual tech review (expect review; open source + reproducible builds should pass — guess). Precedent: TunnelMC spawns ngrok/cloudflared |
| SpigotMC | ~4 MB (moderator, 2020/2021 threads) | upstream BlueMap is already an **external link** (resource 83557); premium may download deps at runtime |
| CurseForge | <2 GB | no external download links |
| Aternos | only installs Spigot/Bukkit-hosted, non-external plugins | unreachable for us, as for any external-link plugin |

Upstream `bluemap-5.28-paper.jar` = 6,285,353 B, so a fat 30 MB jar **does not fit Hangar**. Per-OS jars fit:
shim (<1 MB) + one deflated binary (~5 MB) ≈ 6 MB, the same as upstream. Modrinth allows several files per version,
so we can ship a fat jar and per-OS jars there.

Host constraints (Pterodactyl/Pelican Wings and yolks):
- Images are Ubuntu glibc: focal 2.31 for java8, jammy for 17, noble for 21/25. No Alpine.
- Root fs is read-only. The server dir is a writable bind mount, and executing from it is allowed; non-Java eggs do
  this. `/tmp` is tmpfs `exec`.
- **`container_pid_limit` defaults to 512 and counts threads.** The JVM and the core share it, so cap the core's
  rayon/tokio threads by `render-thread-count` plus a small fixed pool.
- **The child RSS shares the cgroup limit**, and the stock Paper egg runs `-XX:MaxRAMPercentage=95.0`. A core with no
  headroom risks an OOM kill of the whole container. The migration notes must tell users to lower the heap
  percentage, and `/bluemap status` should show core RSS (see Q11).
- Other hosts (Apex, Shockbyte, Bisect…) mostly run Pterodactyl-derived panels (guess). No published child-process
  bans were found.

macOS: files a `java` process writes get no `com.apple.quarantine` xattr, so there is no Gatekeeper prompt. arm64
must be at least ad-hoc signed, which Apple ld/lld do by default. Re-sign (`codesign -s -`) after any `strip`, and
verify the cargo-zigbuild output.

Extraction: `plugins/BlueMap/bin/<target>/bluemap-core-<ver>[.exe]` is written temp+rename, `chmod 755`, and
SHA-256 checked against a manifest in the jar on every start (~20 ms). The versioned filename avoids "exe in use"
on Windows upgrades. Older versions are deleted after a successful start. Keep it out of `/tmp` because of noexec
(00 "Consequences").
Override: a `BLUEMAP_CORE` system property/env var (or hidden `plugin.conf` key) for noexec hosts, unsupported
arches (armv7, FreeBSD, riscv) and debugging. Without an override, unsupported platform or exec failure → loud
error naming the override, plus a pointer to running the core as a standalone CLI.
Linux: ship **static musl** (`x86_64/aarch64-unknown-linux-musl`, `armv7-unknown-linux-musleabihf`, built with
cargo-zigbuild from any host by `tools/build_core.py`) so the glibc version and Alpine stop mattering. TLS is
rustls-ring, so nothing links OpenSSL. Allocator, measured on testbox (forced render of `structures`, optimized
storage, 12 threads, 4 interleaved runs, all cross-built with zig; `docs/perf-exp/testbox_alloc.sh`):

| Build | CPU (user+sys) | Wall | Peak RSS |
|---|---|---|---|
| glibc 2.31 (system malloc) | 49.8 s | 5.4 s | 452 MB |
| musl malloc | 61.8 s | 7.7 s | 544 MB |
| musl + mimalloc v3 defaults | 51.6 s | 6.4 s | 723 MB |
| **musl + mimalloc, purge delay 3 ms, no eager arena commit** (shipped, `bm-cli/src/alloc.rs`) | 57.7 s | 6.9 s | 325 MB |

mimalloc's defaults buy CPU with +60 % RSS, which a core sharing a container with the JVM can't afford; the tuned
setting beats musl malloc on both axes. `MIMALLOC_*` env vars still override it. Compat-storage renders are
byte-identical across all variants (optimized `.bmb` bundles differ in record order between any two renders; see
"Determinism" in `bm-storage/src/optimized/bundle/mod.rs`). The
remaining gap to glibc (+27 % wall) is the price of one portable binary; a `gnu.2.17` build is the fallback if it
matters.
macOS: binaries need at least an ad-hoc signature on arm64 (rustc/ld does this by default). For quarantine, see
the limits note above.

## 6. Webserver

The core keeps the integrated axum server and reads the same `webserver.conf` (`ip` default `0.0.0.0`, `port` 8100,
`sse-enabled`, `additional-headers`, `log`). Nothing moves to Java:
- Port conflicts behave as upstream: bind failure → the same `BindException` explanation (`Plugin.java:277-285`),
  as a `Log` at ERROR. Hosts with port allocations (Pterodactyl) already allocate 8100 for BlueMap, so there is no
  change.
- On a crash restart the core rebinds at once (tokio sets `SO_REUSEADDR` on Unix; Windows does not block on
  TIME_WAIT). A stale core still holding the port is handled by the `.core.lock` kill above.
- The webroot comes from disk with the embedded webapp as fallback, so files that plugins drop under `getWebRoot()`
  (scripts, icons, `createImage`) are served. `registerScript/Style` updates `settings.json` with upstream's 1 s
  debounce (`C/api/WebAppImpl.java:115-140`).
- During reload and `NotReady` the server stays up (upstream `keepWebserver`). It is down only while the core is
  restarting after a crash, ~1 s.

## 7. Decisions (user, 2026-10-07)

All recommended answers below were accepted: plugin `name: BlueMap`; per-OS jars plus a universal jar on Modrinth and
GitHub; Paper + Folia first; refuse to start next to upstream; `setTileFilter` warn-and-ignore; `Logger` facade; own
bStats id; demand-driven live markers; auto-respawn with backoff; armv7-musl if cheap; memory key later.

1. **Jar layout**: Hangar caps uploads at 10 MB, so a fat jar (~30 MB) cannot go there. → *Per-OS jars (~6 MB
   each, same size as upstream) on every venue, plus a fat "universal" jar on Modrinth and GitHub*. Hangar gets
   `linux-x64` as the primary file (guess: the large majority of servers) and the other OS jars as additional
   files or links. Spigot stays an external link, as upstream. No download-on-first-start, because offline hosts,
   Modrinth's "Delphi" review and CurseForge's rules all work against it.
2. **Plugin identity**: same plugin name `BlueMap` (data folder `plugins/BlueMap/`, drop-in for free) or a new name
   with `provides: [BlueMap]` plus migration of the folder? → *Keep `name: BlueMap`* for the data folder and `depend`
   resolution. This ties to the open public-name question in 00. The display name and jar name can differ.
3. **Coexistence**: refuse to start if upstream BlueMap is also installed? → *Yes*, with a clear error, because both
   would claim `BlueMap` and render the same maps.
4. **`setTileFilter`**: two small addons use it (Area Control, MC Map Sync, 3★ each). Implement it as a batched
   per-region upcall (tile list in, bitmask out), or warn and ignore? → *Warn and ignore in v1* (deprecated for
   removal upstream); the upcall is cheap to add later. `getRenderManager` (MC Map Sync) is a plain RPC, so
   support it.
5. **Internals**: Sign Extractor and New Map cast to `BlueMapAPIImpl`/`BlueMapMapImpl` or use core classes, and
   BMUtils logs through core `Logger`. → *Ship a working `core.logger.Logger` facade* (it routes to the plugin
   logger), *provide the impl classes with unwrap methods that throw a clear `UnsupportedOperationException`*,
   and nothing else.
6. **Telemetry**: upstream bStats id 5912 and BlueMap's own metrics POST (`Plugin.java:394-401`) would count our
   installs as BlueMap's. → *Own bStats id, never post to upstream's metrics endpoint*, respecting `metrics=false`.
7. **Live markers when nobody is viewing**: always serialize every 10 s, or demand-driven (`MarkerDemand`)? →
   *Demand-driven*: fewer server CPU spikes than upstream on large claim maps, same visible behaviour.
8. **Core restart policy**: auto-respawn with backoff while keeping the API registered? → *Yes* (5 crashes / 10 min
   cap); a crash does not cycle `onDisable/onEnable` for marker plugins.
9. **Extra targets**: linux-armv7 (32-bit Raspberry Pi OS) and FreeBSD? → *armv7-musl yes if CI is cheap*; FreeBSD
   via the override only.
10. **Spigot (non-Paper) and Folia**: upstream ships a separate Spigot jar and supports Folia. → *Paper + Folia
    first* (Folia: no `SaveWorld`, per-entity scheduler snapshots as upstream). Spigot later via the same shim core.
11. **Memory budget key**: add a hidden `core.conf` key that caps the core's RSS (`#565`, `#271`)? → *Yes, later*.
    Document that container limits now cover JVM + core.

## 8. Implementation status (2026-10-07)

Vertical slice done end to end on Windows and Linux: `crates/bm-ipc` (protocol, crate docs = spec), `bluemap
--plugin-ipc` (`crates/bm-cli/src/plugin/`), the Java shim (`platforms/common` + `platforms/paper`; layout and the
Fabric mod in docs/16), `tools/e2e_paper.py` and the
per-target builds (`tools/build_core.py`, shared with `.github/workflows/{ci,release}.yml`).

**Verified** by `tools/e2e_paper.py` on Paper 26.3 build 159 (Java 25) with BlueBorder 1.1.2 (marker addon)
and BetterStresstestbots (server-side fake players), 43/43 checks on Windows (windows-x64 jar) and on the Linux
testbox (linux-x64 static musl jar, `--core`/`--jar` prebuilt from Windows):
- core spawn → `Ready` 1–4 s with a warm data folder; core RSS ~46–53 MiB after rendering a 3-map spawn-area world.
- webapp, lowres and hires tiles served from `webserver.conf`; `/bluemap`, `maps`, `force-update`, `troubleshoot`,
  `debug world|map|dump`, `storages [<s>]`, `reload` from the console; a bot appears in `live/players.json`;
  BlueBorder's set reaches `live/markers.json` and comes back after reload and after a killed core is respawned;
  server stop leaves no core process and writes `tasks.dat`; SIGKILLing the JVM makes the core exit (stdin EOF).
- Against upstream BlueMap 5.28 Paper on a copy of the same world: `live/markers.json` and the empty
  `live/players.json` are byte-identical; console text of `storages`, `storages file`, `debug world <map> 0 64 0`
  identical; all generated configs identical except the deliberate `format: optimized` block in `storages/*.conf`.
- `--server folia --mc 26.2` (Folia 26.2 build 7; BlueBorder and the bots don't declare `folia-supported`, so no
  marker/player checks) and `--mc 26.2|26.1.2` on Paper (`context` world from each version's vanilla server) pass on
  the Linux testbox, upstream comparison included.
- With every map frozen and the render-threads stopped, the `/bluemap` and `/bluemap maps` console text is identical
  to upstream's.
- Switching back mid-render: ours stops a forced render of `structures` (31 regions, 1 thread) partway; upstream
  loaded on that folder reads our `tasks.dat` and lists the task already past our done regions (`/bluemap maps`).
- Cross-built cores: linux-arm64 and linux-armv7 render `structures` byte-identically to glibc x64 under
  qemu-user; `tasks.dat` encoder byte-identical to BlueNBT 3.5.1 (unit test).
- Rust tests: framing edge cases (`bm-ipc`), players JSON vs `JsonWriter` (`bm-map`), command spec/parser,
  pluginState, scripted-shim lifecycle (`crates/bm-cli/tests/plugin_ipc.rs`: handshake, `NotReady`, command
  round trip, lock exit 4, protocol mismatch exit 3, `Shutdown`/EOF → `Bye`).

Sizes (core stripped of debuginfo; jar = shim + one deflated core; Hangar cap 10,000,000 B):

| Target | Core | Jar |
|---|---|---|
| windows-x64 (MSVC) | 15,448,064 | 6,507,252 |
| linux-x64 (musl) | 14,369,704 | 6,628,116 |
| linux-arm64 (musl) | 13,021,968 | 6,225,810 |
| linux-armv7 (musl) | 12,969,700 | 6,216,263 |
| universal (the four above) | | 24,856,874 |

macOS cores are built only in CI (native `cargo build` on `macos-latest`, `codesign --verify`); cross-building them
from Windows needs the Apple SDK, so they are unverified locally.

**Deviations from the design**
- `Hello` carries `maxMemoryMib` (render-thread suggestion in a new `core.conf`); `Unloading` was added so the shim
  runs the API `onDisable` cycle for reloads it didn't start; `Suggest` is not implemented (the shim suggests from
  `commands.json` + mirrored ids).
- Command tree: one shared `crates/bm-cli/src/plugin/commands.json` (no `--dump-command-tree`); the shim registers
  `bluemap` + a greedy argument and sends the sender's permission nodes, the core gates per usage.
- Shim compiles against paper-api 1.21.11 for Java 21 (26.x API jars are Java 25 class files) and recompiles
  BlueMapAPI 2.8.1 from its sources jar for the same reason. 1.21.x servers should work too (untested).
- The core logs one line per finished render task (`Map 'x': N regions, M tiles rendered, …`); upstream is silent.
- Command output follows the 5.28 command classes' wording, layout and palette (status, maps, tasks, version, help,
  start/stop, freeze/unfreeze, purge, update, cancel) without hover/click events. Task refs are a hash of the task,
  not random; `version` prints `bluemap-rs <version>` where upstream prints its git hash; `start` adds which
  beyond-parity pause still holds (docs/15).
- The core retries a `level.dat`/`world_gen_settings.dat` that fails to decompress (5 × 200 ms): Paper writes level
  data off-thread after the enable-time save, while our core is already loading.
- `reload light` reloads resources too; the rayon pool keeps the first load's thread count until restart; the
  `RenderStart{threads}` count is ignored.
- Marker demand: viewers (SSE or `markers.json` read within 30 s), the first 30 s after a load and 30 s before a
  storage write.
- Skins: the shim runs upstream's `PlayerSkinUpdater` logic (≤1/h per player, API `SkinProvider` +
  `PlayerIconFactory`) and sends one `AssetWrite`/`AssetDelete` of `playerheads/<uuid>.png` per map.
- `tasks.dat` (raw BlueNBT, `renderTasks: [{type, data}]`): whole-map tasks are written as `map-update` with their
  region list at save time (upstream writes an unprepared task as `unknown` and loses it); `map-save` entries are
  ignored on load (we save after every task); unreadable files are logged and deleted as upstream.
- `debug dump` (`commands/dump/`) writes `StateDumper`'s keys, nesting and Java shapes (`#identity` class names,
  `{size, entries}` collections, `<<identity>>` back-references) for what the core holds, checked against an
  upstream 5.28 Paper dump: `system-info` (version, 5.28 `git-hash`, `properties` `os.name`/`user.dir`/
  `file.separator`, cores, max-memory from `Hello`, time), all 15 `registries` with upstream's keys,
  `BlueMapService` (`config` with every core/webserver/webapp/plugin/map/storage config field under its Java
  name, `webFilesManager`, `minecraftVersion`, `worlds`, `maps`, `storages`) and `Plugin` (`pluginState`,
  `renderManager` with tasks and progress tracker, `mapUpdateServices`). JVM-only, so absent: the `java.*`/
  `os.version` properties, `total-`/`free-memory`, `threads` (stack traces), and the reflective depth below
  (resource pack, caches, render state cells, API objects, lambdas). Extras: `bluemap-rs-version`,
  `resident-memory`, our hidden config keys, pause reasons. `debug world` with no map for the sender's world
  answers "No map found" (upstream loads the world on demand).
- Hourly watcher restart (`Plugin.java` `fileWatcherRestartTask`): not done. Upstream closes and recreates every
  `MapUpdateService`; that only drops debounced region updates, resets the update-cooldown cache and leaks the old
  full-update timer, and the new `WatchService` does not rescan. Our watchers already re-create a failed watcher,
  rescan on lost events and every `region-file-check-interval`, so nothing observable is lost.
- Watcher full updates start `full-update-interval` after the map's `lastFullUpdate` (not after load) and record
  the time in `pluginState.json` (next save), as `MapUpdateService.onFullUpdate`; the CLI anchors them at start.
- Beyond upstream: render pausing per reason (stop, players, `memory-limit`, server MSPT via `ServerLoad`, protocol
  2) and low-priority render threads; see docs/15.
- `storages <s> delete <map>` queues `StorageDeleteTask` as a render-queue job (`bm_engine::Job`, ahead of queued
  map updates, cancellable, progress in `/bluemap`); `purge` still runs on the command thread.
- Release jars and cores carry one version: `tools/build_core.py` takes `$BLUEMAP_RS_VERSION`, else the `v*` tag
  (CI) or a `v*` tag on HEAD, else Cargo's, and passes it to cargo (`bluemap --version`, `coreVersion`) and Gradle
  (`-PreleaseVersion` → `5.28+rs.<version>` in plugin.yml/fabric.mod.json, jar names, natives manifest).
- Maps load only per (re)load, as upstream, so every loaded map's web routes exist from session start. (The CLI's
  `-u -w` loads maps whose world appears later; they get live routes through `bm_web::MapRegistry`.)
- musl cores use mimalloc with a 3 ms purge delay (table in §5).

**Left**
- macOS run on real hardware (CI builds and signs only), Folia with players and a marker addon (none of the e2e
  addons load on Folia), player-head bytes vs upstream on a real online-mode join (offline bots have no skin),
  bStats id.
- A world Paper 26.3 generates itself keeps its spawn chunks unlit on disk for the first sessions (even after
  `save-all flush`), so we skip them (upstream wrote no tiles in the same window either); the e2e therefore starts from the lit `context` fixture world.
