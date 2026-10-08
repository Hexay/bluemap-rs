//! `DebugCommand`: `debug dump`, `debug world …` (block, light, biome, chunk at a position), `debug map …`
//! (chunk hash and tile state at a position).

use std::path::PathBuf;
use std::sync::{Arc, PoisonError};

use bm_engine::MapContext;
use bm_ipc::CommandSender;
use bm_world::{BiomeId, ChunkSlot, StateId};
use serde_json::{Value, json};

use super::super::core::{CORE_VERSION, Core};
use super::super::rstate;
use super::super::session::{Session, world_id};
use super::super::text::{self, BASE, NEGATIVE, POSITIVE, hl};
use super::parse::Matched;
use super::{Say, task_ref};
use crate::log;

/// `debug dump` (works unloaded): `StateDumper`'s layout (`system-info`, `registries`, `dump`, `threads`; one-space
/// indent), but `dump` holds a fixed description of the plugin and service state instead of upstream's reflective
/// walk over Java objects, and `registries`/`threads` stay empty (docs/13 §8).
pub fn dump(core: &Core, say: Say) -> i32 {
    let session = core.session();
    let file =
        session.as_ref().map_or_else(|| PathBuf::from("dump.json"), |s| s.service.config.core.data.join("dump.json"));
    let state = dump_json(core, session.as_deref());
    let written = file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&file, gson_pretty(&state)));
    if let Err(e) = written {
        log::error(&format!("Failed to create dump! {e}"));
        say(text::one("Exception trying to create debug-dump! See console for details.", NEGATIVE));
        return 0;
    }
    let path = bm_config::generate::format_path(&file, &std::env::current_dir().unwrap_or_default());
    say(text::lines(text::fill("Dump \u{1F4A9} created at: %", &[hl(&path)], POSITIVE)));
    1
}

/// Gson's `JsonWriter` with `setIndent(" ")`.
fn gson_pretty(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b" "));
    serde::Serialize::serialize(v, &mut ser).expect("JSON values serialize");
    out
}

fn dump_json(core: &Core, s: Option<&Session>) -> Value {
    let mut dump = vec![plugin_json(core, s)];
    dump.extend(s.map(service_json));
    json!({
        "system-info": system_info(core),
        "registries": [],
        "dump": dump,
        "threads": [],
    })
}

/// `collectSystemInfo`, with the JVM's properties replaced by the core process's equivalents.
fn system_info(core: &Core) -> Value {
    let cwd = std::env::current_dir().unwrap_or_default();
    let now = chrono::Local::now();
    json!({
        "bluemap-version": bm_engine::BLUEMAP_VERSION,
        "bluemap-rs-version": CORE_VERSION,
        "properties": {
            "os.name": std::env::consts::OS,
            "os.arch": std::env::consts::ARCH,
            "user.dir": cwd.display().to_string(),
            "platform": core.hello.platform,
            "minecraft.version": core.hello.mc_version,
        },
        "cores": std::thread::available_parallelism().map_or(1, |n| n.get()),
        "max-memory": core.hello.max_memory_mib.map(|m| m << 20),
        "resident-memory": bm_ipc::resident_memory(),
        "timestamp": now.timestamp_millis(),
        "time": now.naive_local().format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
    })
}

/// `Plugin` + its `RenderManager`.
fn plugin_json(core: &Core, s: Option<&Session>) -> Value {
    let state = s.map(|s| s.state.lock().unwrap_or_else(PoisonError::into_inner).to_json());
    let task = |t: &bm_engine::RenderTask| {
        json!({"ref": task_ref(t), "map": t.map, "description": t.description(), "strategy": t.strategy.key()})
    };
    let render_manager = s.map(|s| {
        json!({
            "running": !s.queue.is_paused(),
            "render-threads": rayon::current_num_threads(),
            "current-task": s.queue.current().map(|(description, progress)| json!({
                "description": description,
                "progress": progress,
                "task": s.queue.current_task().as_ref().map(task),
            })),
            "render-tasks": s.queue.pending_tasks().iter().map(task).collect::<Vec<_>>(),
        })
    });
    json!({
        "#identity": "Plugin",
        "loaded": s.is_some_and(Session::is_loaded),
        "loading": core.is_loading(),
        "plugin-state": state.and_then(|j| serde_json::from_str::<Value>(&j).ok()),
        "render-manager": render_manager,
        "server-worlds": *core.worlds.lock().unwrap_or_else(PoisonError::into_inner),
    })
}

/// `BlueMapService`: loaded maps and the configured storages.
fn service_json(s: &Session) -> Value {
    let maps: Vec<Value> = s
        .maps
        .all()
        .iter()
        .map(|m| json!({"id": m.id, "world": world_id(m), "storage": m.config.storage, "warnings": m.warnings}))
        .collect();
    json!({
        "#identity": "BlueMapService",
        "maps": maps,
        "storages": s.service.config.storages.keys().collect::<Vec<_>>(),
        "webroot": s.service.config.webapp.webroot.display().to_string(),
    })
}

/// The first loaded map of the sender's world (`plugin.getWorld(serverWorld)` stands in for a map's world).
fn sender_map(s: &Session, sender: &CommandSender) -> Option<Arc<MapContext>> {
    let world = sender.world.as_deref()?;
    let mut maps: Vec<_> = s
        .maps
        .all()
        .into_iter()
        .filter(|m| s.map_server_world.get(&m.id).cloned().flatten().as_deref() == Some(world))
        .collect();
    maps.sort_by_key(|m| m.config.sorting);
    maps.into_iter().next()
}

fn no_map(say: Say) -> i32 {
    say(text::one("No map found for your world", NEGATIVE));
    0
}

pub fn world(s: &Session, m: &Matched, sender: &CommandSender, say: Say) -> i32 {
    let map = match m.get("map") {
        Some(id) => s.maps.get(id),
        None => sender_map(s, sender),
    };
    let Some(map) = map else { return no_map(say) };
    let pos = match (m.int("x"), m.int("y"), m.int("z"), sender.position) {
        (Some(x), Some(y), Some(z), _) => (x, y, z),
        (_, _, _, Some([x, y, z])) => (x.floor() as i32, (y - 0.1).floor() as i32, z.floor() as i32),
        _ => return no_map(say),
    };
    say(text::paragraph("World-Info (debug)", block_info(&map, pos)));
    1
}

fn block_info(map: &MapContext, (x, y, z): (i32, i32, i32)) -> Vec<Vec<Value>> {
    let world = &map.world;
    let (cx, cz) = (x >> 4, z >> 4);
    let area = world.load_area(cx, cz, 1, 1);
    let chunk = match area.slot(cx, cz) {
        Some(ChunkSlot::Loaded(c)) => Some(c),
        _ => None,
    };
    let state = chunk.map_or(StateId::AIR, |c| c.block(x, y, z));
    let biome = chunk.map_or(BiomeId::DEFAULT, |c| c.biome(x, y, z));
    let (sky, block) = chunk.map_or((0, 0), |c| c.light(x, y, z));
    let n = |v: i64| v.to_string();
    let mut lines =
        text::fill("position: ( x: % | y: % | z: % )", &[hl(&n(x.into())), hl(&n(y.into())), hl(&n(z.into()))], BASE);
    lines.push(text::item("block", &world.states().get(state).key));
    lines.extend(text::details(
        vec![
            vec![text::item("biome", &world.biomes().name(biome))],
            vec![text::item("block-light", &block.to_string())],
            vec![text::item("sky-light", &sky.to_string())],
        ],
        BASE,
    ));
    lines.extend(text::fill("chunk: ( x: % | z: % )", &[hl(&n(cx.into())), hl(&n(cz.into()))], BASE));
    let mut chunk_items = vec![
        vec![text::item("is generated", &chunk.is_some_and(|c| c.generated).to_string())],
        vec![text::item("has lightdata", &chunk.is_some_and(|c| c.has_light).to_string())],
    ];
    if let Some(c) = chunk {
        chunk_items.push(vec![text::item("data-version", &c.data_version.to_string())]);
    }
    chunk_items.push(vec![text::item("inhabited-time", &chunk.map_or(0, |c| c.inhabited_time).to_string())]);
    lines.extend(text::details(chunk_items, BASE));
    lines.push(text::item("world", &world_id(map)));
    lines.extend(text::details(
        vec![
            vec![text::item("min-y", &world.dimension_type.min_y.to_string())],
            vec![text::item("height", &world.dimension_type.height.to_string())],
        ],
        BASE,
    ));
    lines
}

pub fn map(s: &Session, m: &Matched, sender: &CommandSender, say: Say) -> i32 {
    let sender_pos = sender.position.map(|[x, _, z]| (x.floor() as i32, z.floor() as i32));
    let pos = match (m.int("x"), m.int("z")) {
        (Some(x), Some(z)) => Some((x, z)),
        _ => sender_pos,
    };
    let map = match m.get("map") {
        Some(id) => {
            let Some(map) = s.maps.get(id) else { return no_map(say) };
            let explicit = m.int("x").is_some();
            let world = s.map_server_world.get(id).cloned().flatten();
            if !explicit && world != sender.world {
                say(text::lines(text::fill("Map % is not from your current world", &[(id, "")], NEGATIVE)));
                return 0;
            }
            map
        }
        None => match sender_map(s, sender) {
            Some(map) => map,
            None => return no_map(say),
        },
    };
    let Some(pos) = pos else { return no_map(say) };
    say(text::paragraph("Map-Info (debug)", map_info(&map, pos)));
    1
}

fn map_info(map: &MapContext, (x, z): (i32, i32)) -> Vec<Vec<Value>> {
    let (cx, cz) = (x >> 4, z >> 4);
    let tile = map.hires_grid.tile_of(x, z);
    let info = rstate::tile_info(&map.storage, tile);
    let last = rstate::chunk_hash(&map.storage, (cx, cz));
    let current = match map.world.region(x >> 9, z >> 9) {
        Ok(region) => region.timestamp(cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize) as i32,
        Err(e) => {
            if map.world.region_exists(x >> 9, z >> 9) {
                log::error(&format!("Failed to load chunk-hash. {e}"));
            }
            0
        }
    };
    let n = |v: i32| v.to_string();
    let mut lines = text::fill("position: ( x: % | z: % )", &[hl(&n(x)), hl(&n(z))], BASE);
    lines.extend(text::fill("chunk: ( x: % | z: % )", &[hl(&n(cx)), hl(&n(cz))], BASE));
    lines.extend(text::details(
        vec![vec![text::item("current hash", &n(current))], vec![text::item("last rendered", &n(last))]],
        BASE,
    ));
    lines.extend(text::fill("tile: ( x: % | z: % )", &[hl(&n(tile.0)), hl(&n(tile.1))], BASE));
    let mut items = Vec::new();
    if info.render_time > 0 {
        let mut rendered = text::item("rendered", &text::since(info.render_time.into()));
        rendered.push(text::span(" ago", BASE));
        items.push(vec![rendered]);
    }
    items.push(vec![text::item("state", info.state.key())]);
    lines.extend(text::details(items, BASE));
    lines
}
