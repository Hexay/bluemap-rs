use serde::{Deserialize, Serialize};

/// `plugin.conf` (`PluginConfig.java`), server platforms only; serializes with Java's field names.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, rename_all(serialize = "camelCase", deserialize = "kebab-case"))]
pub struct PluginConfig {
    pub live_player_markers: bool,
    /// Java default is empty; the template writes `["spectator"]`.
    pub hidden_game_modes: Vec<String>,
    pub hide_vanished: bool,
    pub hide_invisible: bool,
    pub hide_sneaking: bool,
    pub hide_different_world: bool,
    pub hide_below_sky_light: i32,
    pub hide_below_block_light: i32,
    /// Seconds; <= 0 disables.
    pub write_markers_interval: i32,
    /// Seconds; <= 0 disables.
    pub write_players_interval: i32,
    pub skin_download: bool,
    /// <= 0 disables render pausing.
    pub player_render_limit: i32,
    /// Hidden bluemap-rs keys (docs/15): rendering pauses while the server's 10 s average tick time (ms) is above
    /// `render-pause-mspt` and resumes below `render-resume-mspt`; <= 0 disables.
    pub render_pause_mspt: f64,
    pub render_resume_mspt: f64,
}

impl Default for PluginConfig {
    fn default() -> Self {
        Self {
            live_player_markers: true,
            hidden_game_modes: Vec::new(),
            hide_vanished: true,
            hide_invisible: true,
            hide_sneaking: false,
            hide_different_world: false,
            hide_below_sky_light: 0,
            hide_below_block_light: 0,
            write_markers_interval: 0,
            write_players_interval: 0,
            skin_download: true,
            player_render_limit: -1,
            render_pause_mspt: 45.0,
            render_resume_mspt: 40.0,
        }
    }
}
