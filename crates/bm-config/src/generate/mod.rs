//! First-start config files, rendered from BlueMap's own templates exactly like `BlueMapConfigManager` does.
//! Our only change: generated storage configs get `format: optimized` (`templates/storage-format.conf`).

mod maps;
mod path;

use std::path::Path;

pub(crate) use maps::sanitise_map_id;
pub use maps::{DimensionPreset, ServerWorld, auto_map_configs, default_map_configs, map_conf};
pub use path::format_path;

use crate::template::ConfigTemplate;

/// The BlueMap release whose templates (and `${version}`) we reproduce.
pub const BLUEMAP_VERSION: &str = "5.28";

const CORE: &str = include_str!("../../templates/bluemap/core.conf");
const WEBSERVER: &str = include_str!("../../templates/bluemap/webserver.conf");
const WEBAPP: &str = include_str!("../../templates/bluemap/webapp.conf");
const PLUGIN: &str = include_str!("../../templates/bluemap/plugin.conf");
const FILE_STORAGE: &str = include_str!("../../templates/bluemap/storages/file.conf");
const SQL_STORAGE: &str = include_str!("../../templates/bluemap/storages/sql.conf");
const STORAGE_FORMAT: &str = include_str!("../../templates/storage-format.conf");

/// `getSeparator()` is "/" on every real filesystem (Windows accepts it too).
fn join_formatted(dir: &Path, cwd: &Path, file: &str) -> String {
    format!("{}/{file}", format_path(dir, cwd))
}

/// `core.conf`. `timestamp` is Java's `LocalDateTime.now().withNano(0)` (see [`java_local_date_time`]).
pub fn core_conf(
    data_folder: &Path,
    cwd: &Path,
    use_metrics_config: bool,
    is_cli: bool,
    render_thread_count: i32,
    timestamp: &str,
) -> String {
    let logs = data_folder.join("logs");
    ConfigTemplate::new(CORE)
        .conditional("metrics", use_metrics_config)
        .var("timestamp", timestamp)
        .var("version", BLUEMAP_VERSION)
        // Java reads the minecraftVersion field before the constructor assigns it, so this is always "?"
        .variable("mcVersion", None)
        .var("data", &format_path(data_folder, cwd))
        .var("implementation", "bukkit")
        .var("render-thread-count", &render_thread_count.to_string())
        .var("default-thread-priority", "5")
        .conditional("update-interval-u-flag", is_cli)
        .var("logfile", &format_path(&logs.join("debug.log"), cwd))
        .var("logfile-with-time", &join_formatted(&logs, cwd, "debug_%1$tF_%<tH-%<tM-%<tS.log"))
        .build()
}

/// `webserver.conf`; `data_root` is the loaded `core.conf` `data`.
pub fn webserver_conf(webroot: &Path, data_root: &Path, cwd: &Path) -> String {
    let logs = data_root.join("logs");
    ConfigTemplate::new(WEBSERVER)
        .var("webroot", &format_path(webroot, cwd))
        .var("logfile", &format_path(&logs.join("webserver.log"), cwd))
        .var("logfile-with-time", &join_formatted(&logs, cwd, "webserver_%1$tF_%<tH-%<tM-%<tS.log"))
        .build()
}

pub fn webapp_conf(webroot: &Path, cwd: &Path) -> String {
    ConfigTemplate::new(WEBAPP).var("webroot", &format_path(webroot, cwd)).build()
}

pub fn plugin_conf() -> String {
    ConfigTemplate::new(PLUGIN).build()
}

/// `storages/file.conf`; `webroot` is the loaded `webapp.conf` `webroot`.
pub fn file_storage_conf(webroot: &Path, cwd: &Path) -> String {
    ConfigTemplate::new(FILE_STORAGE).var("root", &format_path(&webroot.join("maps"), cwd)).build() + STORAGE_FORMAT
}

pub fn sql_storage_conf() -> String {
    ConfigTemplate::new(SQL_STORAGE).build() + STORAGE_FORMAT
}

/// BlueMap's render-thread suggestion for a new `core.conf` (deliberately pessimistic). Without a JVM heap limit,
/// `max_memory_mib: None` skips the memory condition.
pub fn suggest_render_thread_count(cores: usize, max_memory_mib: Option<u64>) -> i32 {
    let mem_ok = |mib: u64| max_memory_mib.is_none_or(|m| m >= mib);
    match cores {
        c if c >= 10 && mem_ok(8192) => 3,
        c if c >= 6 && mem_ok(4096) => 2,
        _ => 1,
    }
}

/// Java `LocalDateTime.toString()` without nanos: `yyyy-MM-ddTHH:mm:ss`, seconds omitted when zero.
pub fn java_local_date_time(unix_secs: i64, utc_offset_secs: i32) -> String {
    let t = unix_secs + utc_offset_secs as i64;
    let (days, secs) = (t.div_euclid(86400), t.rem_euclid(86400));
    // civil-from-days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let date = format!("{year:04}-{month:02}-{day:02}T{h:02}:{m:02}");
    if s == 0 { date } else { format!("{date}:{s:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_date_time() {
        assert_eq!(java_local_date_time(0, 0), "1970-01-01T00:00");
        assert_eq!(java_local_date_time(1_791_324_513, 0), "2026-10-06T22:08:33");
        assert_eq!(java_local_date_time(1_791_324_513, -2 * 3600), "2026-10-06T20:08:33");
    }

    #[test]
    fn thread_suggestion() {
        assert_eq!(suggest_render_thread_count(4, None), 1);
        assert_eq!(suggest_render_thread_count(8, Some(2048)), 1);
        assert_eq!(suggest_render_thread_count(8, Some(4096)), 2);
        assert_eq!(suggest_render_thread_count(16, None), 3);
    }
}
