//! `debug dump` (works unloaded): `StateDumper`'s `dump.json` in the data folder, written like Gson's `JsonWriter`
//! with `setIndent(" ")`. Upstream walks the JVM's objects by reflection; this writes the same keys, nesting and
//! Java collection shapes for the state the core holds. What only a JVM has is left out (docs/13 §8).

mod configs;
mod java;
mod plugin;
mod service;

use std::path::PathBuf;

use serde_json::{Value, json};

use super::super::core::{CORE_VERSION, Core};
use super::super::text::{self, NEGATIVE, POSITIVE, hl};
use super::Say;
use crate::log;
use java::Ids;

/// `BlueMap.GIT_HASH` of the 5.28 release whose behaviour and webapp this core reproduces.
const BLUEMAP_GIT_HASH: &str = "0f3a9fbfb87ecfafc809d673b11254634e734ae2";

pub fn dump(core: &Core, say: Say) -> i32 {
    let session = core.session();
    let file =
        session.as_ref().map_or_else(|| PathBuf::from("dump.json"), |s| s.service.config.core.data.join("dump.json"));
    let mut ids = Ids::default();
    let system_info = system_info(&mut ids, core);
    let registries = java::registries(&mut ids);
    let mut objects = Vec::new();
    if let Some(s) = session.as_deref() {
        objects.push(service::blue_map_service(&mut ids, core, s));
    }
    let service = objects.first().and_then(|o| o["#identity"].as_str()).map(str::to_owned);
    objects.push(plugin::plugin(&mut ids, core, session.as_deref(), service.as_deref()));
    let dump = json!({
        "system-info": system_info,
        "registries": registries,
        "dump": objects,
        "threads": [],
    });
    let written = file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&file, gson_pretty(&dump)));
    if let Err(e) = written {
        log::error(&format!("Failed to create dump! {e}"));
        say(text::one("Exception trying to create debug-dump! See console for details.", NEGATIVE));
        return 0;
    }
    let path = bm_config::generate::format_path(&file, &std::env::current_dir().unwrap_or_default());
    say(text::lines(text::fill("Dump \u{1F4A9} created at: %", &[hl(&path)], POSITIVE)));
    1
}

fn gson_pretty(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b" "));
    serde::Serialize::serialize(v, &mut ser).expect("JSON values serialize");
    out
}

/// `System.getProperty("os.name")` for the targets we build.
fn java_os_name() -> &'static str {
    match std::env::consts::OS {
        "linux" => "Linux",
        "macos" => "Mac OS X",
        "windows" => "Windows",
        "freebsd" => "FreeBSD",
        other => other,
    }
}

/// `collectSystemInfo`. The JVM's own properties and heap figures don't exist here; the core adds its version
/// and resident memory at the end.
fn system_info(ids: &mut Ids, core: &Core) -> Value {
    let cwd = std::env::current_dir().unwrap_or_default().display().to_string();
    let separator = std::path::MAIN_SEPARATOR.to_string();
    let known = [("os.name", java_os_name().to_owned()), ("user.dir", cwd), ("file.separator", separator)];
    let keys: Vec<&str> = known.iter().map(|(k, _)| *k).collect();
    let properties = bm_java::hash_map_order(&keys)
        .into_iter()
        .filter_map(|k| known.iter().find(|(key, _)| *key == k))
        .map(|(k, v)| (json!(k), json!(v)))
        .collect();
    let now = chrono::Local::now();
    json!({
        "bluemap-version": bm_engine::BLUEMAP_VERSION,
        "git-hash": BLUEMAP_GIT_HASH,
        "properties": ids.map("java.util.HashMap", properties),
        "cores": std::thread::available_parallelism().map_or(1, |n| n.get()),
        "max-memory": core.hello.max_memory_mib.map(|m| m << 20),
        "timestamp": now.timestamp_millis(),
        "time": now.naive_local().format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
        "bluemap-rs-version": CORE_VERSION,
        "resident-memory": bm_ipc::resident_memory(),
    })
}

#[cfg(test)]
mod tests;
