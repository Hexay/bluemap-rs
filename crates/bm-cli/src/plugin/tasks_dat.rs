//! `<data>/tasks.dat`: the render queue across restarts and reloads (`Plugin.save`/`load`, `TasksData`).
//!
//! Upstream writes it with BlueNBT, uncompressed: root compound `""` → `renderTasks`: list of
//! `{type: "bluemap:<kind>", data: {…}}`, kinds `region-update {map, regionPos: [x, z] ints, force}`,
//! `map-update {map, tasks: [region-update…, map-save], currentTaskIndex}`, `map-save {map}`, `map-purge {map}`
//! and `unknown` (no data, read back as nothing). Entries that fail to load (unknown map, bad data) are skipped;
//! an unreadable file is logged and deleted.
//!
//! Our tasks map back as: one region → `region-update`; several regions or a whole map → `map-update` (a whole
//! map lists its region files at save time, where upstream's not-yet-prepared update would be lost as `unknown`).
//! On load, a `map-update` resumes at `currentTaskIndex`, `map-save` is implied by every task, `map-purge` runs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bm_engine::{Regions, RenderTask, TileUpdateStrategy};
use bm_nbt::Compound;

use super::session::Session;
use super::{ops, spawn};
use crate::log;

pub fn file(data: &Path) -> PathBuf {
    data.join("tasks.dat")
}

/// `Plugin.save()`'s tasks part: the current task first, then the queue.
pub fn save(s: &Session) {
    let tasks: Vec<RenderTask> = s.queue.current_task().into_iter().chain(s.queue.pending_tasks()).collect();
    let bytes = encode(&tasks, &|map| s.maps.get(map).and_then(|m| m.world.regions().ok()));
    let path = file(&s.service.config.core.data);
    let tmp = path.with_extension("dat.filepart");
    let written = std::fs::create_dir_all(&s.service.config.core.data)
        .and_then(|()| std::fs::write(&tmp, &bytes))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = written {
        log::error(&format!("Failed to save tasks.dat! {e}"));
    }
}

/// `Plugin.load()`'s tasks part: queues the saved tasks and runs saved purges.
pub fn resume(s: &std::sync::Arc<Session>) {
    let path = file(&s.service.config.core.data);
    let Ok(bytes) = std::fs::read(&path) else { return };
    let loaded = match decode(&bytes, &|map| s.maps.get(map).is_some()) {
        Ok(l) => l,
        Err(e) => {
            log::error(&format!("Failed to load tasks.dat! {e}"));
            let _ = std::fs::remove_file(&path);
            return;
        }
    };
    for task in loaded.tasks {
        s.queue.schedule(task);
    }
    for map in loaded.purges {
        let s = s.clone();
        spawn("bluemap-purge", move || {
            if let Err(e) = ops::purge(&s, &map) {
                log::error(&format!("Failed to purge map '{map}': {e:#}"));
            }
        });
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Loaded {
    pub tasks: Vec<RenderTask>,
    pub purges: Vec<String>,
}

pub fn decode(bytes: &[u8], map_loaded: &dyn Fn(&str) -> bool) -> bm_nbt::Result<Loaded> {
    let root = bm_nbt::read_root(bytes)?;
    let mut out = Loaded::default();
    let Some(list) = root.list("renderTasks") else { return Ok(out) };
    for entry in list.compounds() {
        let (Some(kind), Some(data)) = (entry.str("type").map(key), entry.compound("data")) else { continue };
        let Some(map) = data.str("map").filter(|m| map_loaded(m)) else { continue };
        match kind.as_str() {
            "bluemap:region-update" => out.tasks.extend(region(&data).map(|(r, f)| only(map, [r].into(), f))),
            "bluemap:map-update" => {
                let start = data.i64("currentTaskIndex").unwrap_or(0).max(0) as usize;
                let subtasks = data.list("tasks").map(|l| l.compounds().skip(start).collect::<Vec<_>>());
                let mut groups: Vec<(TileUpdateStrategy, BTreeSet<(i32, i32)>)> = Vec::new();
                for sub in subtasks.unwrap_or_default() {
                    let is_region = sub.str("type").map(key).as_deref() == Some("bluemap:region-update");
                    let Some((pos, force)) = sub.compound("data").filter(|_| is_region).and_then(|d| region(&d)) else {
                        continue;
                    };
                    match groups.iter_mut().find(|(f, _)| *f == force) {
                        Some((_, set)) => drop(set.insert(pos)),
                        None => groups.push((force, [pos].into())),
                    }
                }
                out.tasks.extend(groups.into_iter().map(|(force, set)| only(map, set, force)));
            }
            "bluemap:map-purge" => out.purges.push(map.to_owned()),
            _ => {}
        }
    }
    Ok(out)
}

fn only(map: &str, regions: BTreeSet<(i32, i32)>, strategy: TileUpdateStrategy) -> RenderTask {
    RenderTask { map: map.to_owned(), regions: Regions::Only(regions), strategy }
}

/// `Key.parse(s, "bluemap").getFormatted()`.
fn key(s: &str) -> String {
    if s.find(':').is_some_and(|i| i > 0) { s.to_owned() } else { format!("bluemap:{s}") }
}

/// `regionPos` and `force` (unknown keys fall back to `force_none`, as `RegistryAdapter` does).
fn region(data: &Compound) -> Option<((i32, i32), TileUpdateStrategy)> {
    let pos: Vec<i64> = data.list("regionPos")?.iter().filter_map(|t| t.as_i64()).collect();
    let [x, z] = pos[..] else { return None };
    let force = data.str("force").and_then(TileUpdateStrategy::from_key).unwrap_or(TileUpdateStrategy::ForceNone);
    Some(((x as i32, z as i32), force))
}

/// Lists a map's region files; `None` if it can't.
pub type RegionLister<'a> = &'a dyn Fn(&str) -> Option<Vec<(i32, i32)>>;

/// `TasksData` as BlueNBT writes it; `regions` lists a map's region files for whole-map tasks.
pub fn encode(tasks: &[RenderTask], regions: RegionLister) -> Vec<u8> {
    let mut w = Nbt::default();
    w.header(COMPOUND, "");
    w.header(LIST, "renderTasks");
    w.list_head(COMPOUND, tasks.len());
    for task in tasks {
        let listed: Option<Vec<(i32, i32)>> = match &task.regions {
            Regions::Only(set) => Some(set.iter().copied().collect()),
            Regions::All => regions(&task.map),
        };
        match listed {
            Some(r) if r.len() == 1 => w.region_update(&task.map, r[0], task.strategy),
            Some(r) => {
                w.typed("bluemap:map-update");
                w.string("map", &task.map);
                w.header(LIST, "tasks");
                w.list_head(COMPOUND, r.len() + 1);
                for &pos in &r {
                    w.region_update(&task.map, pos, task.strategy);
                }
                w.typed("bluemap:map-save");
                w.string("map", &task.map);
                w.end_typed();
                w.header(INT, "currentTaskIndex");
                w.0.extend(0i32.to_be_bytes());
                w.end_typed();
            }
            None => {
                w.string("type", "bluemap:unknown");
                w.0.push(END);
            }
        }
    }
    w.0.push(END);
    w.0
}

const END: u8 = 0;
const INT: u8 = 3;
const STRING: u8 = 8;
const LIST: u8 = 9;
const COMPOUND: u8 = 10;

/// Raw writer: `bm_nbt::Writer` has no int lists and types empty lists `END`, BlueNBT types them by element.
#[derive(Default)]
struct Nbt(Vec<u8>);

impl Nbt {
    fn raw_str(&mut self, s: &str) {
        self.0.extend((s.len() as u16).to_be_bytes());
        self.0.extend(s.as_bytes());
    }

    fn header(&mut self, ty: u8, name: &str) {
        self.0.push(ty);
        self.raw_str(name);
    }

    fn list_head(&mut self, elem: u8, len: usize) {
        self.0.push(elem);
        self.0.extend((len as i32).to_be_bytes());
    }

    fn string(&mut self, name: &str, value: &str) {
        self.header(STRING, name);
        self.raw_str(value);
    }

    /// Opens a list element `{type, data: {`.
    fn typed(&mut self, kind: &str) {
        self.string("type", kind);
        self.header(COMPOUND, "data");
    }

    /// Closes `data` and the element.
    fn end_typed(&mut self) {
        self.0.extend([END, END]);
    }

    fn region_update(&mut self, map: &str, (x, z): (i32, i32), force: TileUpdateStrategy) {
        self.typed("bluemap:region-update");
        self.string("map", map);
        self.header(LIST, "regionPos");
        self.list_head(INT, 2);
        self.0.extend(x.to_be_bytes());
        self.0.extend(z.to_be_bytes());
        self.string("force", force.key());
        self.end_typed();
    }
}

#[cfg(test)]
mod tests;
