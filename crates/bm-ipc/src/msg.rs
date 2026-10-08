//! Typed headers of every message (see the crate docs for the table).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A server world (`ServerWorld`): one per dimension, all sharing the server's level folder on Paper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorldInfo {
    /// Bukkit world key (`minecraft:overworld`); players and `SaveWorld` refer to worlds by it.
    pub id: String,
    pub name: String,
    pub uuid: String,
    /// Absolute level folder.
    pub folder: String,
    /// Dimension key, e.g. `minecraft:the_nether`.
    pub dimension: String,
    /// From the environment; `None` for custom ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dimension_type: Option<String>,
}

/// `BukkitPlayer` as of the last 1 Hz snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerInfo {
    pub uuid: String,
    /// `PlayerDisplayNameProvider` result (default: the account name).
    pub name: String,
    /// `WorldInfo.id`.
    pub world: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    /// Pitch/yaw are Java floats widened to double before sending, as upstream's `Vector3d` does.
    pub pitch: f64,
    pub yaw: f64,
    pub sky_light: i32,
    pub block_light: i32,
    pub sneaking: bool,
    pub invisible: bool,
    pub vanished: bool,
    /// `survival|creative|adventure|spectator`.
    pub gamemode: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSender {
    /// `console|player|other`.
    pub kind: String,
    pub name: String,
    /// `WorldInfo.id` of a player sender.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<[f64; 3]>,
    /// The permission nodes of `commands.json` the sender holds; the core gates each command on them.
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all_fields = "camelCase")]
pub enum ShimMsg {
    Hello {
        protocol: u32,
        platform: String,
        mc_version: String,
        shim_version: String,
        config_folder: String,
        #[serde(default)]
        mods_folder: Option<String>,
        #[serde(default)]
        metrics: Option<bool>,
        #[serde(default)]
        folia: bool,
        #[serde(default)]
        max_memory_mib: Option<u64>,
        worlds: Vec<WorldInfo>,
    },
    WorldAdded {
        world: WorldInfo,
    },
    WorldRemoved {
        id: String,
    },
    Players {
        players: Vec<PlayerInfo>,
    },
    PlayerJoin {
        uuid: String,
    },
    PlayerLeave {
        uuid: String,
    },
    Markers {
        map: String,
        #[serde(default)]
        more: bool,
    },
    Command {
        id: u64,
        input: String,
        sender: CommandSender,
    },
    RenderStart {
        id: u64,
        #[serde(default)]
        threads: Option<i32>,
    },
    RenderStop {
        id: u64,
    },
    RenderStatus {
        id: u64,
    },
    Schedule {
        id: u64,
        map: String,
        #[serde(default)]
        regions: Option<Vec<[i32; 2]>>,
        #[serde(default)]
        force: bool,
    },
    Purge {
        id: u64,
        map: String,
    },
    SetFrozen {
        id: u64,
        map: String,
        frozen: bool,
    },
    SetPlayerVisibility {
        id: u64,
        uuid: String,
        visible: bool,
    },
    RegisterScript {
        id: u64,
        url: String,
    },
    RegisterStyle {
        id: u64,
        url: String,
    },
    AssetWrite {
        id: u64,
        map: String,
        name: String,
    },
    AssetRead {
        id: u64,
        map: String,
        name: String,
    },
    AssetExists {
        id: u64,
        map: String,
        name: String,
    },
    AssetDelete {
        id: u64,
        map: String,
        name: String,
    },
    Reload {
        id: u64,
        #[serde(default)]
        light: bool,
    },
    ServerLoad {
        mspt: f64,
    },
    Shutdown,
    Reply(Reply),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    pub id: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub err: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub value: Value,
}

impl Reply {
    pub fn ok(id: u64, value: Value) -> Self {
        Self { id, ok: true, err: None, value }
    }

    pub fn err(id: u64, err: impl Into<String>) -> Self {
        Self { id, ok: false, err: Some(err.into()), value: Value::Null }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreWorld {
    /// BlueMap's `World.id`: `<folder relative to the server dir>#<dimension>`.
    pub id: String,
    /// The dimension folder (`BlueMapWorld.getSaveFolder`), absolute.
    pub save_folder: String,
    /// The matching `WorldInfo.id`, if the server has that world loaded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_world: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MapInfo {
    pub id: String,
    pub name: String,
    /// `CoreWorld.id`.
    pub world: String,
    pub tile_size: [i32; 2],
    pub tile_offset: [i32; 2],
    pub frozen: bool,
    /// `MarkerGson` JSON of the map config's `marker-sets`, loaded into the API's sets before `onEnable`.
    pub config_markers: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub live_player_markers: bool,
    pub skin_download: bool,
    pub player_render_limit: i32,
    pub metrics: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyInfo {
    pub core_version: String,
    pub compat_version: String,
    /// Absolute webroot (`WebApp.getWebRoot`).
    pub webroot: String,
    pub worlds: Vec<CoreWorld>,
    pub maps: Vec<MapInfo>,
    pub storages: Vec<String>,
    pub plugin: PluginInfo,
    pub state: StateInfo,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateInfo {
    pub frozen_maps: Vec<String>,
    pub hidden_players: Vec<String>,
    pub render_threads_running: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all_fields = "camelCase")]
pub enum CoreMsg {
    Welcome {
        protocol: u32,
        core_version: String,
        compat_version: String,
        pid: u32,
    },
    Incompatible {
        protocol: u32,
        core_version: String,
    },
    Ready(Box<ReadyInfo>),
    NotReady {
        reason: String,
        message: String,
    },
    Unloading,
    StateChanged(StateInfo),
    Log {
        level: LogLevel,
        msg: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trace: Option<String>,
    },
    CommandOutput {
        id: u64,
        component: Value,
    },
    CommandDone {
        id: u64,
        result: i32,
    },
    SaveWorld {
        id: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        world: Option<String>,
    },
    MarkerDemand {
        maps: Vec<String>,
    },
    Reply(Reply),
    Bye,
}

#[cfg(test)]
mod tests;
