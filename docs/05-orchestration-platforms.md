# 05 — Orchestration, config, platforms, public API

Path prefixes used below (all inside the cloned sources):
- `C/` = `BlueMap/common/src/main/java/de/bluecolored/bluemap/common/`
- `RES/` = `BlueMap/common/src/main/resources/de/bluecolored/bluemap/config/`
- `CORE/` = `BlueMap/core/src/main/java/de/bluecolored/bluemap/core/`
- `IMPL/<p>/` = `BlueMap/implementations/<p>/src/main/java/de/bluecolored/bluemap/<pkg>/`
- `API/` = `BlueMapAPI/src/main/java/de/bluecolored/bluemap/api/`

Toolchain: both repos build with Java 25 (`buildSrc/src/main/kotlin/bluemap.java.gradle.kts:34`, `BlueMapAPI/buildSrc/.../bluemap.base.gradle.kts:35`). So every supported server already runs Java ≥ 22 → Panama FFM is available everywhere BlueMap runs.

## 1. Lifecycle

Platform entrypoint creates `new Plugin("<impl>", serverInterface)` and calls `load()` on a **separate thread** after server start (`IMPL/paper/BukkitPlugin.java:140`, `IMPL/fabric/FabricMod.java:103-117`). `onDisable`/`SERVER_STOPPING` → `unload()`.

`Plugin.load()` (`C/plugin/Plugin.java:129-433`), guarded by an interruptible lock:
1. Load **addons** from `<config>/packs/*.jar` (`:138-140`) — before configs.
2. Load configs via `BlueMapConfigManager` (writes defaults if missing; auto-generates one map config per loaded server world) (`:143-151`).
3. Optional file logger (`:158-166`).
4. Load `<data>/pluginState.json` (render-threads on/off, per-map `updateEnabled`/`lastFullUpdate`, hidden players) (`:176-184`, `C/plugin/PluginState.java:36-88`).
5. `BlueMapService` → `getOrLoadResourcePack()` — downloads MC client jar if `accept-download`; else `MissingResourcesException` → unload and wait for `/bluemap reload` (`:187-205`).
6. `getOrLoadMaps()` → per map: load World (MCA + datapacks) once per (folder, dimension), Storage once per storage id, `BmMap`, static marker sets from config (`C/BlueMapService.java:149-246`).
7. Webserver (if enabled): routes `.*`→static webroot, `maps/<id>/(.*)`→`MapRequestHandler` (live players/markers JSON + SSE) (`Plugin.java:214-287`).
8. No maps → unload but keep webserver (`:290-294`).
9. `new RenderManager()`; write webapp files + `settings.json` (`:297-301`).
10. Skin updater listener; `Timer` jobs: save every 10 min (+ clear `ArrayTileModel` pool), markers/players write intervals, **restart file-watchers hourly**, metrics every 30 min (`:304-401`).
11. Schedule full update for maps whose `lastFullUpdate + full-update-interval` has passed (`:361-374`).
12. Restore queued tasks from `<data>/tasks.dat` (NBT) (`:377-390`).
13. Start `MapUpdateService` (watcher) per non-frozen map; register join/leave listener; register API instance (fires `BlueMapAPI.onEnable` consumers); start render threads unless paused (`:404-419`).

`unload()` (`:439-541`): unregister API → listeners → timer → watchers → `save()` (tasks.dat, pluginState.json, each map) → cancel current task, `awaitIdle`, stop threads → close webserver (unless `keepWebserver`) → close storages.
`reload()` = unload+load; `lightReload()` keeps the parsed ResourcePack (`:551-571`).

CLI (`IMPL/cli/BlueMapCLI.java`) skips `Plugin` entirely: addons → configs (`usePluginConfig(false)`, `isCli(true)`, data=`data`, webroot=`web`) → `BlueMapService` → actions (`:393-452`). Exit codes: 1 config/IO error, 2 missing resources (`:466-491`).

## 2. Render manager

`C/rendermanager/RenderManager.java` — one `LinkedList<RenderTask>` queue, N worker threads, **all workers cooperate on the head task** (`doWork` takes `getFirst()`, `:303-349`). Task is removed only when `!hasMoreWork()` and `busyCount==0` (`:318-334`). Last 10 completed kept for status (`:66-71`). Worker exception → sleep 10 s, continue (`:370-382`). Progress/ETA via `ProgressTracker(5000ms,12)`.

`RenderTask` interface (`RenderTask.java:29-64`): `doWork()` (called concurrently, task self-partitions), `hasMoreWork()`, `estimateProgress()`, `cancel()`, `contains(other)` (dedup), `getDescription()`.

Queueing semantics:
- `scheduleRenderTask` → append; rejects if an existing non-head task `contains` it; removes queued tasks that the new one contains (cancels head if contained) (`:142-151`, `:293-301`).
- `scheduleRenderTaskNext` → insert at index 1 (right after the running one) (`:163-173`). Used for full updates and `/bluemap update`.
- `reorderRenderTasks`, `removeRenderTasksIf` (freeze/purge), `removeAllRenderTasks` (shutdown).
No priority queue beyond "append vs next".

Task types:
| Task | Ser. key | What |
|---|---|---|
| `MapUpdatePreparationTask` | — | single-shot: list region files (bounds+radius filter), add regions from map region-state if `check-for-removed-regions`; builds `MapUpdateTask` = `[MapSaveTask, WorldRegionUpdateTask… sorted by region lastUpdated then distance to 0,0, MapSaveTask]`, hands it to a consumer (append) (`MapUpdatePreparationTask.java:68-163`). Empty region list → skip (safety against wiping a map). |
| `MapUpdateTask` (extends `CombinedRenderTask`) | `bluemap:map-update` | sequential subtask list, serializes `currentTaskIndex` (`MapUpdateTask.java:37-75`, `CombinedRenderTask.java:49-63`). |
| `WorldRegionUpdateTask` | `bluemap:region-update` | per region (32×32 chunks). `init()` reads region header chunk timestamps ("chunkHashes"), resolves per-tile action via `TileState.findActionAndNextState(changed‖force, bounds)`; preloads region if ≥75 % tiles render. Then each `doWork` takes next tile (x,z) → RENDER/DELETE/NONE, checks preconditions (chunk error, not generated, missing light, min-inhabited-time + radius). `complete()` writes chunk-hash and region-state, `map.saveDebounced()` (`WorldRegionUpdateTask.java:91-264`, `:345-388`). Equality = (map id, region, force). |
| `MapSaveTask` | `bluemap:map-save` | `map.save()` once (`MapSaveTask.java:44-49`). |
| `MapPurgeTask` | `bluemap:map-purge` | discard lowres, `storage.delete(progress)`, reset texture gallery + tile/chunk/region state (`MapPurgeTask.java:51-71`). |
| `StorageDeleteTask` | not serialized | delete a map from a storage (`/bluemap storages … delete`). |
| `CombinedRenderTask` | — | generic sequential container. |

`TileUpdateStrategy` registry: `force_all`, `force_edge` (only `RENDERED_EDGE` tiles), `force_none` (`TileUpdateStrategy.java:39-47`).

Serialization: `tasks.dat` = BlueNBT `{renderTasks:[{type:"bluemap:<key>", data:{…}}]}`; unknown types written as `bluemap:unknown` and skipped; lenient list adapter drops failing entries (`serialization/RenderTaskAdapter.java:48-106`, `Plugin.java:606-622`). Maps are referenced by id (`BmMapAdapter`). Port to Rust: serde enum; on-disk compat optional (only resumes queue).

Thread count: `render-thread-count` (>0 literal; ≤0 → cores+value, min 1) (`C/config/CoreConfig.java:67-70`); default template value suggested by `suggestRenderThreadCount()` = 1/2/3 based on cores & heap (`C/config/BlueMapConfigManager.java:148-158`). Priority 1-10 → `Thread.setPriority`.
Pause: `player-render-limit` — stop threads when online ≥ limit, re-check 1 s after join/leave (`Plugin.java:704-733`).

### Incremental-update triggers (server AND CLI `-u` are identical)
`C/plugin/MapUpdateService.java` — one thread per map, **purely file-based; no platform save/chunk events**:
- `java.nio.file.WatchService` on the dimension's `region/` dir, CREATE/MODIFY/DELETE (`CORE/world/mca/MCAWorldRegionWatchService.java:95-106`) → `updateRegion` debounce: delay = max(cooldown − sinceLast, 5 s), re-armed on each event (`MapUpdateService.java:145-159`) → `WorldRegionUpdateTask` appended.
- Poll `region-file-check-interval` (default 5 min): fingerprint each region file, changed/removed → `updateRegion` (`:188-221`).
- `full-update-interval` (default 1440 min): `MapUpdatePreparationTask` scheduled next (`:110-117`, `:176-186`).
- Plugin restarts all watchers hourly (`Plugin.java:350-358`).
- The only server hook used for freshness: `/bluemap update` calls `ServerWorld.persistWorldChanges()` (force world save on main thread) before scheduling (`C/commands/commands/UpdateCommand.java:161`, `IMPL/paper/BukkitWorld.java:63-76`, `IMPL/fabric/FabricWorld.java:57-80`).

CLI flags (`BlueMapCLI.java:494-554`): `-c` config dir (default `config`), `-n` mods dir, `-v` mc version, `-l`/`-a` log file/append, `-r` render, `-f` force (FORCE_ALL), `-e` fix-edges (FORCE_EDGE), `-m a,b` map filter, `-u` **watch** (render then keep watching), `-w` webserver, `-b` verbose web log, `-g` (re)generate webapp, `-s` update settings.json, `--markers` write config markers to storage, `-V`, `-h`. Render without `-u` exits when queue idle (`:233-246`). CLI progress log every 10 s, map save every 2 min (`:152-188`). API registered with `plugin=null` → no RenderManager/Plugin API (`:140`, `C/api/BlueMapAPIImpl.java:67-69,156,162`).

## 3. Configuration

Format: HOCON via SpongePowered **Configurate** (`.conf`); `.json` also accepted for every file (`C/config/ConfigLoader.java:40-48`). Keys are kebab-case mapped to camelCase fields. Default files are generated from templates with `${var}` and `${cond<<…>>}` blocks (`C/config/ConfigTemplate.java`, `BlueMapConfigManager.java:112-226`). Map id = config filename with `\W`→`_` (`:313,390`). Storage config loaded twice: base for `storage-type`, then concrete class (`:375-376`).

Config root: plugin data folder (Paper), `config/bluemap` (Fabric/Forge), `-c` (CLI). Files:

**core.conf** (`RES/core.conf`, `C/config/CoreConfig.java:38-53`)
`accept-download=false` · `data="bluemap"` (CLI `data`) · `render-thread-count=1` (template: suggested) · `render-thread-priority=5` · `update-cooldown=60` s · `full-update-interval=1440` min · `region-file-check-interval=5` min · `scan-for-mod-resources=true` · `metrics=true` (omitted if platform decides) · `log{file, append=false}`.

**webserver.conf** (`RES/webserver.conf`, `C/config/WebserverConfig.java:41-69`)
`enabled=true` · `webroot="bluemap/web"` · `ip="0.0.0.0"` (hidden) · `port=8100` · `sse-enabled=true` · `additional-headers{Cache-Control: "public, max-age=86400, stale-if-error=604800", CDN-Cache-Control: "max-age=60"}` · `log{file, append=false, format="%1$s \"%3$s %4$s %5$s\" %6$s %7$s"}` (Java `String.format` positional syntax — must emulate).

**webapp.conf** (`RES/webapp.conf`, `C/config/WebappConfig.java:41-68`)
`enabled=true` · `webroot` · `update-settings-file=true` · `use-cookies=true` · `default-to-flat-view=false` · `start-location` (null) · `min/max-zoom-distance=5/100000` · `resolution-default=1` · `hires-slider-max/default/min=500/100/0` · `lowres-slider-max/default/min=7000/2000/500` · `map-data-root="maps"` · `live-data-root="maps"` · `client-decompression=false` · `scripts=[]` · `styles=[]`.

**plugin.conf** (servers only; `RES/plugin.conf`, `C/config/PluginConfig.java:39-54`)
`live-player-markers=true` · `hidden-game-modes=["spectator"]` · `hide-vanished=true` · `hide-invisible=true` · `hide-sneaking=false` · `hide-below-sky-light=0` · `hide-below-block-light=0` · `hide-different-world=false` · `write-markers-interval=0` · `write-players-interval=0` · `skin-download=true` · `player-render-limit=-1`.

**maps/<id>.conf** (`RES/maps/map.conf`, `C/config/MapConfig.java:57-103`)
`loader=bluemap:anvil` (hidden) · `world` (null ⇒ display-only map served from storage) · `dimension` (null ⇒ legacy inference from `DIM-1`/`DIM1`/`dimensions/ns/val`, `C/BlueMapService.java:179-201`) · `dimension-type` · `name` · `sorting=0` · `start-pos{x,z}` · `sky-color="#7dabff"` · `void-color="#000000"` · `sky-light=1` · `ambient-light=0` · `remove-caves-below-y=55` · `cave-detection-ocean-floor=10000` (template writes -5) · `cave-detection-uses-block-light=false` · `min-inhabited-time=0` · `min-inhabited-time-radius=0` (hidden) · `render-mask=[…]` (mask types `box|circle|ellipse|polygon|blur`, `subtract`, min/max xyz; `C/config/mask/MaskType.java:35-39`) · `render-edges=true` · `edge-light-strength=15` (template 8) · `enable-perspective-view/flat-view/free-flight-view/hires=true` · `check-for-removed-regions=true` (hidden) · `storage="file"` · `ignore-missing-light-data=false` · `marker-sets{}` (HOCON→JSON→MarkerGson, `MapConfig.java:108-125`) · hidden `hires-tile-size=32`, `lowres-tile-size=500`, `lod-count=3`, `lod-factor=5`.
Per-dimension template presets: nether sky `#290000`, ambient 0.6, caves off, ceiling mask y90-127 subtract; end sky `#080010` (`BlueMapConfigManager.java:394-437`). Auto-config: one file per server world, id from folder name, sorting 0/100/200/300 (`:256-301`); CLI/no worlds → `overworld`, `nether`, `end` for `world/`.

**storages/file.conf**: `storage-type=file` · `root="bluemap/web/maps"` · `compression=gzip|zstd|deflate|none` · `atomic=true` (hidden) (`C/config/storage/FileConfig.java:40-42`).
**storages/sql.conf**: `connection-url` (JDBC URL) · `connection-properties{user,password}` · `max-connections=-1` · `driver-jar` · `driver-class` · `dialect` (auto from URL: mysql/mariadb/postgresql/sqlite) · `connection-init-sql` · `table-prefix="bluemap_"` · `compression=gzip` (`C/config/storage/SQLConfig.java:55-68`, `Dialect.java:42-59`). Rust: parse JDBC URL → sqlx/rusqlite; `driver-jar` becomes meaningless.

Runtime state files in `data/`: `pluginState.json`, `tasks.dat`, `resourceExtensions.zip` (copied from jar each load), `defaultBlockstates.zip` (built from server registry), downloaded MC client jar, logs.

## 4. Platform layer

The whole contract is `C/serverinterface/*` (~250 lines):
- `Server` (`Server.java:43-131`): `getMinecraftVersion()`, `getConfigFolder()`, `getModsFolder()`, `isMetricsEnabled()` (tristate), `getLoadedServerWorlds()`, `getServerWorld(Object)` (API lookup by name/UUID/native world), `getOnlinePlayers(): Map<UUID,Player>`, **`getDefaultBlockstates()`** (registry dump → `defaultBlockstates.zip` pack; needed for modded blocks), `registerListener/unregisterAllListeners`.
- `ServerWorld` (`ServerWorld.java:34-56`): `getWorldFolder()`, `getDimension()`, `getDimensionType()`, `persistWorldChanges()`.
- `Player` (`Player.java:32-88`): uuid, name, world, position, rotation, sky/block light, sneaking, invisible, vanished, gamemode — **snapshotted on the server thread ~1×/s** (Paper: per-player scheduler every 20 ticks `IMPL/paper/BukkitPlugin.java:286-290`; Fabric: round-robin in END_SERVER_TICK `IMPL/fabric/FabricMod.java:254-271`).
- `ServerEventListener`: only `onPlayerJoin/Leave(uuid)` (`ServerEventListener.java:29-35`) — used for skins and render pause.
- `CommandSource`: `sendMessage(adventure Component)`, `hasPermission`, optional world/position (`CommandSource.java:33-47`). Commands are one shared BlueCommands tree bridged to Brigadier (`C/commands/Commands.java:62-122`); Paper registers via lifecycle event, Fabric via `CommandRegistrationCallback`; Spigot/Sponge have their own bridges (`IMPL/spigot/BukkitCommands.java`, `IMPL/sponge/SpongeCommands.java`).

Platform modules are thin: 6–7 files, ~700–900 lines each (largest `BukkitPlugin.java` 293, `FabricMod.java` 273). Paper also saves all worlds once at enable so `level.dat` exists (`BukkitPlugin.java:103-108`) and has Folia guards (`FoliaSupport.java`). Forge/NeoForge are near copies of each other. Paper/Fabric also run bStats.

What a Rust core therefore needs from a Java shim: (a) config/mods/world folder paths + MC version, (b) list of worlds (folder, dimension key, dimension type), (c) default-blockstate registry dump once at load, (d) player snapshots (~1 Hz), join/leave events, (e) "save world now" RPC, (f) command dispatch + permission checks + message output, (g) the BlueMapAPI surface (below). World data itself is read from disk — Rust never needs live chunk access.

### Integration options
| Option | Fit | Notes |
|---|---|---|
| JNI (`jni-rs`) | works on Java 25 | Most boilerplate; native lib per OS/arch in the jar; a Rust panic/segfault kills the MC server. |
| **Panama FFM** (Java 22+) | best in-process option | BlueMap already requires Java 25 → no compatibility cost. `jextract` from a C ABI header (`cbindgen`); upcalls for player snapshot/save callbacks. Same crash-blast-radius and per-platform native packaging as JNI. Some hosts forbid `--enable-native-access` warnings / native libs (shared hosting). |
| Sidecar process + IPC | robust | Java plugin spawns `bluemap-rs` binary, talks over localhost socket/stdin (JSON/MsgPack). Crash isolation, independent memory (render memory no longer inflates MC heap — a real win). Costs: process supervision, binary distribution per OS, some hosts disallow spawning processes. |
| Rust standalone + thin Java plugin | simplest core | Rust = CLI-equivalent daemon (already file-watcher driven, same as plugin). Java plugin only pushes players/markers/commands over HTTP/IPC. Equivalent to sidecar but user-managed lifecycle. |

Recommendation: design the Rust core with a transport-agnostic "host" trait (the 7 needs above); ship CLI/standalone first, then the sidecar (IPC) shim; FFM in-process later only if needed. Rendering is already 100 % file-based so nothing in the hot path needs the JVM.

### API compatibility problem
Third-party plugins (markers: towns, claims, warps…) call `BlueMapAPI` in-JVM. The Java shim must keep shipping `de.bluecolored.bluemap.api` and implement it by proxying:
- Markers are plain mutable POJOs in a `ConcurrentHashMap`, **no change events** (`API/markers/MarkerSet.java:36-39`); BlueMap itself just polls `MarkerGson.toJson(markerSets)` every 10 s (`C/live/LiveMarkersDataSupplier.java:42-44`, `C/web/MapRequestHandler.java:91`). → Shim can do the same: serialize with `MarkerGson` and push JSON to Rust on change. Cheap and faithful.
- `RenderManager` API (`API/RenderManager.java:42-107`): schedule update/purge/regions, queue size, start/stop → simple RPCs.
- `WebApp.registerScript/registerStyle/createImage/setPlayerVisibility` (`C/api/WebAppImpl.java:67-121`) → RPC (settings.json rewrite, hidden-players in pluginState).
- `AssetStorage` read/write/delete per map, URL `maps/<id>/assets/<name>` (`C/api/AssetStorageImpl.java:48-73`) → stream bytes over IPC.
- `Plugin` API: `SkinProvider`, `PlayerIconFactory(BufferedImage)`, `PlayerDisplayNameProvider` — JVM callbacks; keep skin fetching + head rendering in the shim (Java) and push PNG bytes, or call back over IPC.
- **Hard ones**: `BlueMapMap.setTileFilter(Predicate<Vector2i>)` is evaluated in the render hot path (`CORE/map/BmMap.java:141-142`) — needs a per-tile upcall or a pre-computed tile set; `setFrozen`. And `BlueMapAPIImpl.blueMapService()/plugin()` intentionally expose internals to addons (`C/api/BlueMapAPIImpl.java:182-202`) — unportable.
- `BlueMapAPI.onEnable/onDisable` reload semantics must be preserved by the shim.

## 5. Addons

`C/addons/AddonLoader.java`: scans `<config>/packs/*.jar` for `bluemap.addon.json` `{id, entrypoint, dependencies, soft-dependencies}` (`AddonInfo.java:46-80`), topo-sorts, loads each into a `URLClassLoader` whose parent is **BlueMap's own classloader** (+ dependency loaders), instantiates entrypoint, calls `run()` if `Runnable` (`:144-193`). Loaded once per JVM (static `INSTANCE`), before configs. Same `packs/` dir is also a resource/datapack root (`C/BlueMapService.java:358-367`), so addon jars double as resource packs.
Addons get full access to core internals: static registries — `BlockRendererType`, `EntityRendererType`, `BlockColorCalculatorType`, `BlockEntityType`, `EntityType`, `WorldLoaderType`, `RegionType`, `StorageType`, `Dialect`, `MaskType`, `Compression`, `RenderPassType`, `GrassColorModifier`, atlas `SourceType` (grep `Registry<…> REGISTRY`).
Implication: **Java addons cannot run against a Rust core.** Options: drop them; reimplement known popular ones natively; define a new extension ABI (Rust `cdylib`/WASM plugins, or data-driven JSON for renderer/colour mappings). Resource-pack-only parts of addon jars keep working (zip reading).

## 6. Port difficulty

Ports easily (pure orchestration / data):
- RenderManager + task types + dedup/`contains` logic + tasks.dat (→ serde) — straightforward; replace Java monitor `wait/notify` with a Mutex+Condvar or rayon per-tile parallelism.
- MapUpdateService → `notify` crate + debounce timers; fingerprint poll; full-update scheduler (tokio).
- Plugin lifecycle/reload/timers, PluginState JSON, player-render-limit pause.
- Config: HOCON via `hocon` crate or custom; templates; masks; defaults. Watch out for Java `String.format` in log paths/formats and Configurate quirks (comments, unquoted keys, includes unused).
- Webapp settings.json writer, SSE broadcasting, live players JSON (`C/live/LivePlayersDataSupplier.java:56-102`), metrics POST.
- CLI argument surface (clap).

JVM-bound (stay in the Java shim or need redesign):
- Platform layers (`IMPL/*`), Brigadier/Adventure commands & permissions, player snapshots, `persistWorldChanges`, default-blockstate registry dump.
- BlueMapAPI implementation + marker POJOs/MarkerGson, `PlayerIconFactory`/`SkinProvider` (`BufferedImage`), `setTileFilter` predicate.
- Addon jar loading (Java classloading).
- SQL storage `driver-jar`/`driver-class` JDBC loading (Rust: native drivers for the 4 dialects).
- `StateDumper`/`@DebugDump` reflection debug dumps (`C/debug/StateDumper.java`) → replace with explicit serde dump.
- Memory tricks (`ArrayTileModel.instancePool().clear()`, `Plugin.java:319-320`) — irrelevant in Rust.
