//! `<data>/tasks.dat`: the render queue across restarts and reloads (`Plugin.save`/`load`, `TasksData`), also
//! used by the CLI to resume an interrupted `-f` (docs/15 §4).
//!
//! Upstream writes it with BlueNBT, uncompressed: root compound `""` → `renderTasks`: list of
//! `{type: "bluemap:<kind>", data: {…}}`, kinds `region-update {map, regionPos: [x, z] ints, force}`,
//! `map-update {map, tasks: [region-update…, map-save], currentTaskIndex}`, `map-save {map}`, `map-purge {map}`
//! and `unknown` (no data, read back as nothing). Entries that fail to load (unknown map, bad data) are skipped;
//! an unreadable file is logged and deleted.
//!
//! Our tasks map back as: one region → `region-update`; several regions or a whole map → `map-update` (a whole
//! map lists its region files at save time, where upstream's not-yet-prepared update would be lost as `unknown`).
//! A partly done task lists its done regions first with `currentTaskIndex` past them, so Java resumes it too.
//! On load, a `map-update` resumes at `currentTaskIndex`, `map-save` is implied by every task, `map-purge` runs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use bm_engine::{Regions, RenderTask, TileUpdateStrategy};
use bm_nbt::Compound;

pub fn file(data: &Path) -> PathBuf {
    data.join("tasks.dat")
}

/// Written next to the target and renamed over it.
pub fn write(data: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let path = file(data);
    let tmp = path.with_extension("dat.filepart");
    std::fs::create_dir_all(data)?;
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, &path)
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
    RenderTask::new(map, Regions::Only(regions), strategy)
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

/// What one task is written as: its regions (done ones first) and how many are done; `None` = `unknown`.
type Listed = Option<(Vec<(i32, i32)>, usize)>;

fn listed(task: &RenderTask, regions: RegionLister) -> Listed {
    let all: Vec<(i32, i32)> = match &task.regions {
        Regions::Only(set) => set.iter().copied().collect(),
        Regions::All => regions(&task.map)?,
    };
    let (mut done, left): (Vec<_>, Vec<_>) = all.into_iter().partition(|r| task.done.contains(r));
    let index = done.len();
    done.extend(left);
    Some((done, index))
}

/// `TasksData` as BlueNBT writes it; `regions` lists a map's region files for whole-map tasks. Tasks with every
/// region done are left out.
pub fn encode(tasks: &[RenderTask], purges: &[String], regions: RegionLister) -> Vec<u8> {
    let entries: Vec<(&RenderTask, Listed)> = tasks
        .iter()
        .map(|t| (t, listed(t, regions)))
        .filter(|(_, l)| l.as_ref().is_none_or(|(r, done)| *done < r.len()))
        .collect();
    let mut w = Nbt::default();
    w.header(COMPOUND, "");
    w.header(LIST, "renderTasks");
    w.list_head(COMPOUND, entries.len() + purges.len());
    for (task, listed) in entries {
        match listed {
            Some((r, 0)) if r.len() == 1 => w.region_update(&task.map, r[0], task.strategy),
            Some((r, done)) => {
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
                w.0.extend((done as i32).to_be_bytes());
                w.end_typed();
            }
            None => {
                w.string("type", "bluemap:unknown");
                w.0.push(END);
            }
        }
    }
    for map in purges {
        w.typed("bluemap:map-purge");
        w.string("map", map);
        w.end_typed();
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
