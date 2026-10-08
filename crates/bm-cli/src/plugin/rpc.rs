//! BlueMapAPI calls the shim forwards (`RenderManagerImpl`, `BlueMapMapImpl.setFrozen`, `WebAppImpl`,
//! `AssetStorageImpl`). Each runs on its own thread and answers with one `Reply`.

use std::collections::BTreeSet;
use std::sync::PoisonError;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use bm_engine::{Regions, RenderTask};
use bm_ipc::{CoreMsg, Reply, ShimMsg};
use bm_storage::ItemKey;
use serde_json::{Value, json};

use super::core::Core;
use super::ops;
use super::session::Session;

/// Handles an RPC message; `false` if `msg` isn't one.
pub fn handle(core: &Core, msg: ShimMsg, body: Vec<u8>) {
    let (id, result) = match msg {
        ShimMsg::RenderStart { id, .. } => (id, with_session(core, |s| render(core, s, true))),
        ShimMsg::RenderStop { id } => (id, with_session(core, |s| render(core, s, false))),
        ShimMsg::RenderStatus { id } => (id, with_session(core, status)),
        ShimMsg::Schedule { id, map, regions, force } => {
            (id, with_session(core, |s| schedule(s, &map, regions, force)))
        }
        ShimMsg::Purge { id, map } => (id, with_session(core, |s| ops::purge(s, &map).map(|()| Value::Null))),
        ShimMsg::SetFrozen { id, map, frozen } => {
            (id, with_session(core, |s| Ok(json!(ops::set_frozen(core, s, &map, frozen)))))
        }
        ShimMsg::SetPlayerVisibility { id, uuid, visible } => (
            id,
            with_session(core, |s| {
                s.state.lock().unwrap_or_else(PoisonError::into_inner).set_hidden(&uuid, !visible);
                core.state_changed(s);
                Ok(Value::Null)
            }),
        ),
        ShimMsg::RegisterScript { id, url } => (id, register(core, url, true)),
        ShimMsg::RegisterStyle { id, url } => (id, register(core, url, false)),
        ShimMsg::AssetWrite { id, map, name } => (
            id,
            with_session(core, |s| {
                s.service.map_storage(&map)?.write_item(&ItemKey::asset(&name), &body)?;
                Ok(Value::Null)
            }),
        ),
        ShimMsg::AssetRead { id, map, name } => {
            let read = with_session(core, |s| {
                let stored = s.service.map_storage(&map)?.read_item(&ItemKey::asset(&name))?;
                Ok(stored.map(|s| s.decompress()).transpose()?)
            });
            match read {
                Ok(Some(bytes)) => core.out.send_with_body(CoreMsg::Reply(Reply::ok(id, json!(true))), bytes),
                Ok(None) => core.out.send(CoreMsg::Reply(Reply::ok(id, json!(false)))),
                Err(e) => core.out.send(CoreMsg::Reply(Reply::err(id, format!("{e:#}")))),
            }
            return;
        }
        ShimMsg::AssetExists { id, map, name } => {
            (id, with_session(core, |s| Ok(json!(s.service.map_storage(&map)?.item_exists(&ItemKey::asset(&name))?))))
        }
        ShimMsg::AssetDelete { id, map, name } => (
            id,
            with_session(core, |s| {
                s.service.map_storage(&map)?.delete_item(&ItemKey::asset(&name))?;
                Ok(Value::Null)
            }),
        ),
        _ => return,
    };
    core.out.send(CoreMsg::Reply(match result {
        Ok(value) => Reply::ok(id, value),
        Err(e) => Reply::err(id, format!("{e:#}")),
    }));
}

fn with_session<T>(core: &Core, f: impl FnOnce(&Session) -> Result<T>) -> Result<T> {
    let s = core.loaded().ok_or_else(|| anyhow!("BlueMap is not loaded"))?;
    f(&s)
}

fn render(core: &Core, s: &Session, running: bool) -> Result<Value> {
    ops::set_render_threads(core, s, running);
    Ok(Value::Null)
}

fn status(s: &Session) -> Result<Value> {
    let queued = s.queue.pending() + usize::from(s.queue.current_task().is_some());
    Ok(json!({"running": !s.queue.is_paused(), "threads": rayon::current_num_threads(), "queueSize": queued}))
}

fn schedule(s: &Session, map: &str, regions: Option<Vec<[i32; 2]>>, force: bool) -> Result<Value> {
    s.maps.get(map).context("map is not loaded")?;
    let regions = match regions {
        None => Regions::All,
        Some(r) => Regions::Only(r.into_iter().map(|[x, z]| (x, z)).collect::<BTreeSet<_>>()),
    };
    Ok(json!(s.queue.schedule(RenderTask::new(map, regions, ops::strategy(force)))))
}

/// `WebAppImpl.registerScript/Style`: settings.json is rewritten after a 1 s debounce (see `timers`).
fn register(core: &Core, url: String, script: bool) -> Result<Value> {
    crate::log::info(&format!("Registering {} from API: {url}", if script { "script" } else { "style" }));
    let mut files = core.web_files.lock().unwrap_or_else(PoisonError::into_inner);
    let list = if script { &mut files.0 } else { &mut files.1 };
    if !list.contains(&url) {
        list.push(url);
    }
    core.settings_due.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert(Instant::now());
    Ok(Value::Null)
}
