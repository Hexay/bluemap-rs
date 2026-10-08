use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

use crate::de::size;

/// `core.conf` (`CoreConfig.java`). Defaults are the Java field initialisers, not the template values.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct CoreConfig {
    pub accept_download: bool,
    /// > 0: thread count; <= 0: available cores minus this (see [`CoreConfig::resolve_render_thread_count`]).
    pub render_thread_count: i32,
    pub render_thread_priority: i32,
    /// Seconds.
    pub update_cooldown: i32,
    /// Minutes; 0 disables.
    pub full_update_interval: i32,
    /// Minutes; 0 disables.
    pub region_file_check_interval: i32,
    pub metrics: bool,
    pub data: PathBuf,
    pub scan_for_mod_resources: bool,
    pub log: LogConfig,
    /// Hidden bluemap-rs key: bytes the core may use (`512M`, `2G`, …); `None` (unset or 0) = no limit (docs/15).
    #[serde(deserialize_with = "size::opt_memory_size")]
    pub memory_limit: Option<u64>,
}

/// `log { file, append }`; `file` is a Java `String.format` pattern (e.g. `%1$tF`), passed through verbatim.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct LogConfig {
    pub file: Option<String>,
    pub append: bool,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            accept_download: false,
            render_thread_count: 1,
            render_thread_priority: 5,
            update_cooldown: 60,
            full_update_interval: 1440,
            region_file_check_interval: 5,
            metrics: true,
            data: PathBuf::from("bluemap"),
            scan_for_mod_resources: true,
            log: LogConfig::default(),
            memory_limit: None,
        }
    }
}

fn non_negative(v: i32) -> u64 {
    v.max(0) as u64
}

impl CoreConfig {
    pub fn resolve_render_thread_count(&self, available_cores: usize) -> usize {
        if self.render_thread_count > 0 {
            return self.render_thread_count as usize;
        }
        (available_cores as i64 + self.render_thread_count as i64).max(1) as usize
    }

    pub fn update_cooldown_duration(&self) -> Duration {
        Duration::from_secs(non_negative(self.update_cooldown))
    }

    pub fn full_update_interval_duration(&self) -> Duration {
        Duration::from_secs(non_negative(self.full_update_interval) * 60)
    }

    pub fn region_file_check_interval_duration(&self) -> Duration {
        Duration::from_secs(non_negative(self.region_file_check_interval) * 60)
    }
}
