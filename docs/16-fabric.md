# 16 — Fabric mod and the shared shim (`platforms/`)

Design of the server side: docs/13 (Paper first). This doc covers the split into a loader-neutral shim and the Fabric
mod. Upstream reference: BlueMap `v5.28` (`0f3a9fb`), `implementations/fabric`.

## 1. Decisions (user, 2026-10-07)

1. Fabric v1 = **dedicated servers only**. In a client (singleplayer, LAN) the mod logs one line and does nothing.
2. **Minecraft 26.1–26.3** in one jar (Java 25), compiled against 26.1 like upstream 5.28. No 1.21.x Fabric build.
3. Both shims report version **`5.28+rs.<crate version>`** (`platforms/gradle.properties` `bluemapVersion` + the
   workspace `Cargo.toml` version), so addon checks like `bluemap >=5` pass. Fabric mod id `bluemap`; Paper plugin
   name stays `BlueMap`.
4. One Gradle build under `platforms/`: `common` (Java 21, loader-neutral), `paper` (Java 21), `fabric` (Java 25).
5. Shared code logs through SLF4J (Paper: the plugin's SLF4J logger; Fabric: `LoggerFactory`).
6. The IPC protocol is not forked: Fabric needed no new message.
7. NeoForge/Forge later; `common` stays free of loader APIs so they can follow.

## 2. Layout

```
platforms/
  settings.gradle.kts, build.gradle.kts   version scheme; gradle wrapper (run Gradle on JDK 25)
  buildSrc/NativeJars.kt                  per-target + universal jars, natives/manifest.json (shared by both shims)
  natives/<target>/                       core binaries staged by tools/build_core.py (git-ignored)
  common/   bluemaprs.shim.*              IPC client, CoreSupervisor/CoreBinary, API backend + de.bluecolored proxy,
                                          markers, skins, CommandSpec; plus:
            Platform                      what the shim needs from a server (worlds, blockstates, save, config folder)
            ShimCore<S>                   wiring + Hello; one per server run
            PlayerRegistry/PlayerSnapshot 1 Hz Players batch, join/leave frames; platforms push snapshots
            CommandBridge<S>              Brigadier `/bluemap <greedy>` over any command source, via Sender adapter
            Slf4jLogger                   upstream `Logger.global` facade → SLF4J
            BlueMapAPI 2.8.1              recompiled for Java 21 from its sources jar, shipped unrelocated
  paper/    BlueMapPaperPlugin, PaperPlatform, PaperPlayers (entity scheduler), PaperSender, WorldEvents, Coexistence
  fabric/   BlueMapFabricMod, FabricPlatform, FabricPlayers, FabricSender, Coexistence, ClientNotice
```

Jars: `platforms/{paper,fabric}/build/libs/bluemap-rs-<shim>-<crate>-<target|universal>.jar`;
`build/shim/` holds the jar without natives (dev runs set `BLUEMAP_CORE`). Build all: `py -3 tools/build_core.py --jars`.

Fabric build: no-remap Loom `net.fabricmc.fabric-loom` 1.18.3, loader 0.18.4, Fabric API 0.144.0+26.1 (compile).
`:common` classes and the BlueMapAPI jar are copied into the mod jar; fabric-permissions-api 0.7.0 and flow-math
1.0.3 are nested (`include`, jar-in-jar). Gson, Brigadier and SLF4J come from Minecraft.

## 3. Fabric behaviour

| Concern | Fabric | Upstream 5.28 Fabric |
|---|---|---|
| Entry | `server` entrypoint (`DedicatedServerModInitializer`); `client` entrypoint only logs | `main`, runs in clients too |
| Start | `SERVER_STARTED`: `saveAllChunks` once (as Paper/NeoForge, makes `level.dat` current), then the core spawns on `BlueMap-Load` | `SERVER_STARTED`, no save |
| Stop | `SERVER_STOPPING`: `Shutdown`, waits ≤ 30 s for the core; `SaveWorld` then answers `false` (server thread is blocked) | `unload()` |
| Worlds | every `ServerLevel`: id = name = uuid = dimension key, folder = level folder, `dimensionType` from the level's type key | same folder/key |
| Worlds added later | `ServerLevelEvents.LOAD/UNLOAD` → `WorldAdded/Removed` | not forwarded |
| `SaveWorld` | `level.save(null, true, false)` on the server thread | same |
| Blockstates | every `BuiltInRegistries.BLOCK` entry (modded too), `id[k=v,…]` sorted | same set |
| Players | `END_SERVER_TICK` round-robin, `max(1, n/20)` per tick; head yaw; no vanish source | same |
| Join/leave | `ServerPlayConnectionEvents`; every 20 ticks players missing from the player list leave (Carpet bots fire no `DISCONNECT`) | events only |
| Commands | `CommandRegistrationCallback`; nodes via fabric-permissions-api, default op level `MODERATORS` | same |
| Output | core JSON → `ComponentSerialization.CODEC` → `sendSystemMessage`; multi-line framed by newlines | same |
| Folders | config `config/bluemap` (core binary in `config/bluemap/bin/<target>/`), mods `mods` | same |

**Client inertness.** `environment: "*"` keeps the jar loadable in a modpack shared with clients and keeps addon
`depends: bluemap` resolvable there; the server code lives behind the `server` entrypoint, which Fabric never calls
in a client, so a client (incl. its integrated server) never extracts or spawns the core.

**Coexistence with upstream.** Fabric Loader does *not* refuse two jars with mod id `bluemap`: it silently loads one
(the e2e saw it pick upstream 5.28 over `5.28+rs.0.1.0`). If it picks ours, `Coexistence` finds upstream's
`FabricMod.class` in `mods/` and the mod stays inactive with an error; if it picks upstream, ours never runs. Either
way the two never render the same maps.

## 4. Deviations

- Startup world save (Paper and NeoForge do it; upstream Fabric doesn't).
- `WorldAdded/Removed` for dimensions loaded after start.
- Stale-player sweep every 20 ticks (above).
- `/bluemap version` adds `bluemap-rs Fabric shim <version>`.
- Dedicated servers only (decision 1).

## 5. Verified

`py -3 tools/e2e_fabric.py` (Windows, windows-x64 jar): Fabric Loader 0.19.5 + Fabric API 0.162.0+26.3 on MC 26.3,
Carpet 26.3 (fake players), BlueMap Offline Player Markers 2026.9.1 (a Fabric BlueMapAPI addon that `depends` on
`bluemap`). Checks: core → `Ready`; `/bluemap`, `version`, `maps`, `force-update`, `reload` from the console; the addon
gets `onEnable` and registers its script; webapp, lowres and hires tiles served; a bot appears in and leaves
`live/players.json`; a killed core respawns; stop leaves no core and writes `tasks.dat`; killing the JVM ends the
core; with upstream's Fabric jar next to ours no core spawns (23/23). `py -3 tools/e2e_paper.py` still passes 43/43.

## 6. Left

- Singleplayer / LAN (integrated server): per-session `ShimCore`, firewall prompt for a versioned exe, bind address.
- Permission nodes with a real permissions mod (LuckPerms Fabric) and as a non-op player: untested (no client in e2e).
- 26.1 and 26.2 runtime (compiled against 26.1, e2e runs 26.3 only); Linux run of the Fabric e2e.
- Our `Coexistence` stand-down path: in the e2e the loader always picked upstream, so it never ran.
- Carpet bots can't exercise the addon's offline marker (no `DISCONNECT`, no saved player data).
- NeoForge: ModDevGradle + `jarJar`, a `neoforge/` module on `common` (commands: moderator level, as upstream).
