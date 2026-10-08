//! The `Plugin` object: plugin state, render manager and update watchers.

use std::sync::PoisonError;

use bm_engine::{Regions, RenderTask};
use serde_json::{Value, json};

use super::super::super::core::Core;
use super::super::super::session::Session;
use super::configs::COMMON;
use super::java::{Ids, seen};

fn task(ids: &mut Ids, t: &RenderTask) -> Value {
    let class = if t.regions == Regions::All { "MapUpdateTask" } else { "WorldRegionUpdateTask" };
    ids.object(
        &format!("{COMMON}.rendermanager.{class}"),
        vec![("#toString", json!(t.description())), ("map", json!(t.map)), ("updateStrategy", json!(t.strategy.key()))],
    )
}

/// `Instant.toString` of epoch seconds.
pub fn instant(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0).map_or_else(String::new, |t| t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// `Duration.toString` of whole seconds (`PT24H`, `PT1H1M30S`, `PT0S`).
pub fn duration(secs: u64) -> String {
    if secs == 0 {
        return "PT0S".into();
    }
    let mut out = String::from("PT");
    for (v, unit) in [(secs / 3600, 'H'), (secs / 60 % 60, 'M'), (secs % 60, 'S')] {
        if v > 0 {
            out += &format!("{v}{unit}");
        }
    }
    out
}

fn java_value(ids: &mut Ids, class: &str, to_string: String) -> Value {
    ids.object(class, vec![("#toString", json!(to_string))])
}

fn render_manager(ids: &mut Ids, s: &Session, running: bool) -> Value {
    let mut tasks: Vec<Value> = s.queue.current_task().iter().map(|t| task(ids, t)).collect();
    tasks.extend(s.queue.pending_tasks().iter().map(|t| task(ids, t)));
    let (last_progress, samples) = s.progress.lock().unwrap_or_else(PoisonError::into_inner).samples();
    let samples = ids.list("java.util.LinkedList", samples.into_iter().map(Value::from).collect());
    let tracker = ids.object(
        &format!("{COMMON}.rendermanager.ProgressTracker"),
        vec![
            ("averagingCount", json!(crate::eta::AVERAGING)),
            ("lastProgress", json!(last_progress)),
            ("timesPerProgress", samples),
        ],
    );
    let workers = ids.list("java.util.concurrent.ConcurrentLinkedDeque", vec![json!("bluemap-render")]);
    let tasks = ids.list("java.util.LinkedList", tasks);
    let pauses: Vec<String> = s.queue.pause_reasons().iter().map(|r| format!("{r:?}")).collect();
    ids.object(
        &format!("{COMMON}.rendermanager.RenderManager"),
        vec![
            ("running", json!(running)),
            ("workerThreads", workers),
            ("progressTracker", tracker),
            ("renderTasks", tasks),
            ("pauseReasons", json!(pauses)),
        ],
    )
}

fn update_services(ids: &mut Ids, s: &Session, last_full_update: impl Fn(&str) -> i64) -> Value {
    let watched: Vec<String> = s.watchers.lock().unwrap_or_else(PoisonError::into_inner).keys().cloned().collect();
    let core = &s.service.config.core;
    let services = watched
        .into_iter()
        .map(|id| {
            let last = java_value(ids, "java.time.Instant", instant(last_full_update(&id)));
            let every = java_value(ids, "java.time.Duration", duration(core.full_update_interval_duration().as_secs()));
            let cooldown = java_value(ids, "java.time.Duration", duration(core.update_cooldown_duration().as_secs()));
            let service = ids.object(
                &format!("{COMMON}.plugin.MapUpdateService"),
                vec![
                    ("map", json!(id)),
                    ("lastFullUpdate", last),
                    ("fullUpdateInterval", every),
                    ("regionUpdateCooldown", cooldown),
                    ("closed", json!(false)),
                ],
            );
            (json!(id), service)
        })
        .collect();
    ids.map("java.util.HashMap", services)
}

/// `Plugin` with `blueMap` pointing at the dumped `BlueMapService`.
pub fn plugin(ids: &mut Ids, core: &Core, s: Option<&Session>, service: Option<&str>) -> Value {
    let mut fields = vec![("implementationType", json!(core.hello.platform.to_lowercase()))];
    fields.push(("blueMap", service.map_or(Value::Null, seen)));
    if let Some(s) = s {
        let state = s.state.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let maps = state
            .maps
            .iter()
            .map(|(id, m)| {
                let ms = ids.object(
                    &format!("{COMMON}.plugin.PluginState$MapState"),
                    vec![("updateEnabled", json!(m.update_enabled)), ("lastFullUpdate", json!(m.last_full_update))],
                );
                (json!(id), ms)
            })
            .collect();
        let maps = ids.map("java.util.LinkedHashMap", maps);
        let hidden = ids.list("java.util.LinkedHashSet", state.hidden_players.iter().map(|p| json!(p)).collect());
        let plugin_state = ids.object(
            &format!("{COMMON}.plugin.PluginState"),
            vec![
                ("renderThreadsEnabled", json!(state.render_threads_enabled)),
                ("maps", maps),
                ("hiddenPlayers", hidden),
            ],
        );
        let render_manager = render_manager(ids, s, state.render_threads_enabled);
        let services = update_services(ids, s, |id| state.maps.get(id).map_or(0, |m| m.last_full_update));
        fields.extend([
            ("pluginState", plugin_state),
            ("renderManager", render_manager),
            ("mapUpdateServices", services),
        ]);
    }
    fields.extend([("loaded", json!(s.is_some_and(Session::is_loaded))), ("loading", json!(core.is_loading()))]);
    fields.push(("serverWorlds", json!(*core.worlds.lock().unwrap_or_else(PoisonError::into_inner))));
    ids.object(&format!("{COMMON}.plugin.Plugin"), fields)
}
