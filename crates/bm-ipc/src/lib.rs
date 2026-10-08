//! The wire protocol between a server-plugin shim (JVM) and the `bluemap` core process (docs/13 §4). This
//! docstring is the canonical spec; the Java side (`platforms/common`) mirrors it.
//!
//! # Transport
//!
//! The shim spawns `bluemap --plugin-ipc --parent-pid <jvm pid>` with cwd = the server folder and talks over the
//! child's stdin (shim→core) and stdout (core→shim). The core moves its real stdout to a private handle first and
//! points fd 1 at stderr ([`take_stdout`]), so stray prints can't corrupt frames. Everything on the core's stderr
//! (panics, C-library noise) is forwarded by the shim line by line at WARN.
//!
//! # Framing
//!
//! ```text
//! u32 BE  frame_len    = 4 + header_len + body_len
//! u32 BE  header_len
//! [header_len]  header: UTF-8 JSON object; "t" names the message
//! [body_len]    body: raw bytes, may be empty
//! ```
//!
//! Header ≤ [`MAX_HEADER`], body ≤ [`MAX_BODY`]; a peer that receives more treats the stream as corrupt and closes
//! it. Field names are camelCase. Unknown fields are ignored; an unknown `t` is logged and skipped.
//!
//! # Handshake and lifecycle
//!
//! 1. Shim → `Hello` (first frame; body = default-blockstate dump, see below).
//! 2. Core → `Welcome` if `Hello.protocol == PROTOCOL`, else `Incompatible` and exit code 3. Shim and core ship
//!    in one jar, so versions must match exactly.
//! 3. Core loads (configs, resources, maps, webserver), then sends `Ready` or `NotReady`. Only after one of them
//!    are RPCs answered (earlier ones wait).
//! 4. `Reload` repeats step 3 and then replies. `NotReady` keeps the webserver up when it was started.
//! 5. `Shutdown` or stdin EOF: the core unloads like upstream (`pluginState.json`, map saves, players `{}`), sends
//!    `Bye` and exits 0. Lost parent (watchdog on `--parent-pid`) does the same.
//!
//! The core holds an exclusive lock on `<config folder>/.core.lock` while running and writes its pid to
//! `<config folder>/.core.pid`; a second core exits with code 4. A shim that finds a live pid there kills it before
//! spawning (two cores on one map corrupt render state).
//!
//! # Requests
//!
//! Messages with an `id` (u64, unique per sender) are requests; the peer answers `Reply{id, ok, err?, value?}`
//! (`value` = any JSON, plus a body where noted). Both sides issue requests (core→shim: `SaveWorld`).
//!
//! # Shim → core
//!
//! | `t` | fields | body | notes |
//! |---|---|---|---|
//! | `Hello` | `protocol, platform, mcVersion, shimVersion, configFolder, modsFolder?, metrics?, folia, maxMemoryMib, worlds: [WorldInfo]` | default blockstates: JSON object `{"minecraft:stone": "minecraft:stone", …}` (Bukkit `Material` → `BlockData.getAsString()`), or empty | `metrics`: `null` = core.conf decides (Paper) |
//! | `WorldAdded` | `world: WorldInfo` | | |
//! | `WorldRemoved` | `id` | | |
//! | `Players` | `players: [PlayerInfo]` | | 1 Hz, complete batch in the shim's `ConcurrentHashMap<UUID,…>` order (= upstream's JSON order) |
//! | `PlayerJoin` / `PlayerLeave` | `uuid` | | core re-checks `player-render-limit` 1 s later |
//! | `Markers` | `map, more` | `MarkerGson` JSON of the map's marker sets (UTF-8) | bodies > 1 MiB are split; `more: true` = continued in the next `Markers` frame of the same map |
//! | `Command` | `id, input, sender: CommandSender` | | `input` without the leading `/`, e.g. `bluemap freeze world`; answered with `CommandOutput`* then `CommandDone` (no `Reply`) |
//! | `RenderStart` | `id, threads?` | | `RenderManager.start([threads])` |
//! | `RenderStop` | `id` | | |
//! | `RenderStatus` | `id` | | reply value `{running, threads, queueSize}` |
//! | `Schedule` | `id, map, regions?: [[x,z]], force` | | `scheduleMapUpdateTask`; no regions = whole map; reply value `bool` |
//! | `Purge` | `id, map` | | `scheduleMapPurgeTask` |
//! | `SetFrozen` | `id, map, frozen` | | |
//! | `SetPlayerVisibility` | `id, uuid, visible` | | persisted in `pluginState.json` |
//! | `RegisterScript` / `RegisterStyle` | `id, url` | | settings.json rewritten after 1 s (debounced) |
//! | `AssetWrite` | `id, map, name` | asset bytes | |
//! | `AssetRead` | `id, map, name` | | reply value `bool` (found); body = decompressed bytes |
//! | `AssetExists` / `AssetDelete` | `id, map, name` | | reply value `bool` / null |
//! | `Reload` | `id, light` | | reply after the new `Ready`/`NotReady` |
//! | `ServerLoad` | `mspt` | | 1 Hz: the server's average tick time in ms (Paper `getAverageTickTime`, Fabric `getAverageTickTimeNanos`); not sent where unknown (Folia). The core pauses rendering on its 10 s average (`render-pause-mspt`, docs/15) |
//! | `Shutdown` | | | |
//! | `Reply` | `id, ok, err?, value?` | | answers a core request |
//!
//! # Core → shim
//!
//! | `t` | fields | notes |
//! |---|---|---|
//! | `Welcome` | `protocol, coreVersion, compatVersion, pid` | |
//! | `Incompatible` | `protocol, coreVersion` | then exit 3 |
//! | `Ready` | [`ReadyInfo`] | maps = loaded maps (with a world), in `sorting` order |
//! | `NotReady` | `reason: missing-resources\|config-error\|no-maps, message` | API stays unregistered |
//! | `Unloading` | | a reload (command or `Reload`) starts: the shim unregisters the API now (`onDisable` consumers still see it working) and registers a fresh instance on the next `Ready` |
//! | `StateChanged` | [`StateInfo`] | after anything that changes frozen maps, hidden players or render threads |
//! | `Log` | `level: debug\|info\|warning\|error, msg, trace?` | `"N log lines dropped"` summarises overflow |
//! | `CommandOutput` | `id, component` | `component`: vanilla text-component JSON |
//! | `CommandDone` | `id, result` | Brigadier result code |
//! | `SaveWorld` | `id, world?` | `world` = `WorldInfo.id`, absent = all; reply value `bool` (saved); Folia replies `false` |
//! | `MarkerDemand` | `maps: [id]` | maps whose markers the core needs now (viewers, pending writes); on a newly demanded map the shim pushes at once, then every 10 s while demanded and changed |
//! | `Reply` | `id, ok, err?, value?` | |
//! | `Bye` | | last frame before exit |
//!
//! # Commands
//!
//! `crates/bm-cli/src/plugin/commands.json` lists every `/bluemap` usage with its permission node (upstream's
//! command tree). The shim registers `bluemap` (allowed with any of the nodes) with a greedy argument, suggests
//! from that file plus the mirrored map/storage ids, and sends the nodes the sender holds in
//! `CommandSender.permissions`; the core parses the input and checks the node of the usage it matched.
//!
//! `Players`, `Markers` and `ServerLoad` are latest-wins on the sender (a stale batch may be dropped); everything else is
//! delivered in order. Text is UTF-8 everywhere.

mod frame;
mod msg;
mod process;

pub use frame::{Frame, IpcError, MAX_BODY, MAX_HEADER, MarkerAssembler, read_frame, write_frame};
pub use msg::*;
pub use process::{
    CoreLock, LockError, ignore_interrupts, lower_thread_priority, parent_alive, resident_memory, take_stdout,
    wait_for_parent_exit,
};

/// Bumped on every incompatible change (2: `ServerLoad`).
pub const PROTOCOL: u32 = 2;

/// `Markers` bodies are split into frames of at most this size, so player batches aren't held up.
pub const MARKER_CHUNK: usize = 1 << 20;
