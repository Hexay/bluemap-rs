//! Commands that change state: reload, start/stop, freeze/unfreeze, purge, update, tasks cancel.

use std::collections::BTreeSet;
use std::sync::PoisonError;

use bm_engine::{PauseReason, Regions, RenderTask, TileUpdateStrategy};
use bm_ipc::CommandSender;

use super::super::core::Core;
use super::super::ops;
use super::super::session::Session;
use super::super::text::{self, BASE, NEGATIVE, POSITIVE};
use super::parse::Matched;
use super::{Say, info, task_ref};

/// Region files are 512×512 blocks.
const REGION_SHIFT: i32 = 9;

pub fn reload(core: &Core, light: bool, say: Say) -> i32 {
    say(text::one("Reloading BlueMap...", BASE));
    if core.reload(light) {
        say(text::one("BlueMap reloaded!", POSITIVE));
        1
    } else {
        say(text::one("Could not load BlueMap! See logs or console for details!", NEGATIVE));
        0
    }
}

/// `start` clears `/bluemap stop` and the player limit (upstream); memory and server-load pauses are reported.
pub fn start_stop(core: &Core, s: &Session, start: bool, say: Say) -> i32 {
    let enabled = s.state.lock().unwrap_or_else(PoisonError::into_inner).render_threads_enabled;
    let reasons = s.queue.pause_reasons();
    let upstream_paused = reasons.contains(PauseReason::Stopped) || reasons.contains(PauseReason::PlayerLimit);
    let already = if start { enabled && !upstream_paused } else { !enabled };
    if already {
        let reasons = info::pause_lines(core, s);
        if start && !reasons.is_empty() {
            say(headed("Render-Threads are paused:", NEGATIVE, reasons));
        } else {
            let msg = if start { "Render-Threads are already running!" } else { "Render-Threads are already stopped!" };
            say(text::one(msg, NEGATIVE));
        }
        return 0;
    }
    ops::set_render_threads(core, s, start);
    say(text::one(if start { "Render-Threads started!" } else { "Render-Threads stopped!" }, POSITIVE));
    let still = info::pause_lines(core, s);
    if start && !still.is_empty() {
        say(headed("...but they stay paused:", BASE, still));
    }
    1
}

fn headed(header: &str, color: &str, lines: Vec<Vec<serde_json::Value>>) -> serde_json::Value {
    text::lines([vec![text::span(header, color)]].into_iter().chain(lines).collect())
}

pub fn freeze(core: &Core, s: &Session, map: &str, frozen: bool, say: Say) -> i32 {
    if !ops::set_frozen(core, s, map, frozen) {
        let msg = if frozen { "Map % is already frozen" } else { "Map % is not frozen" };
        say(text::lines(vec![text::format(msg, &[map])]));
        return 0;
    }
    let msg = if frozen {
        "Map % is now frozen and will no longer be automatically updated"
    } else {
        "Map % is no longer frozen and will update automatically"
    };
    say(text::lines(vec![text::format(msg, &[map])]));
    1
}

pub fn purge(s: &Session, map: &str, say: Say) -> i32 {
    say(text::lines(vec![text::format("Scheduled a new task to purge map %", &[map])]));
    match ops::purge(s, map) {
        Ok(()) => {
            say(text::lines(vec![text::format("Map % has been purged and will be rendered again", &[map])]));
            1
        }
        Err(e) => {
            crate::log::error(&format!("Failed to purge map '{map}': {e:#}"));
            say(text::lines(vec![text::format(
                "There was an error trying to purge %, see console for details.",
                &[map],
            )]));
            0
        }
    }
}

pub fn cancel_tasks(s: &Session, task: Option<&str>, say: Say) -> i32 {
    let removed = match task {
        None => s.queue.remove_where(|_| true),
        Some(r) => s.queue.remove_where(|t| task_ref(t) == r),
    };
    let msg = match (task, removed) {
        (None, _) => "All tasks cancelled",
        (Some(_), 0) => "Task is not pending or already completed",
        (Some(_), _) => "Task cancelled",
    };
    say(text::one(msg, if removed > 0 || task.is_none() { POSITIVE } else { NEGATIVE }));
    i32::from(removed > 0)
}

/// `UpdateCommand`: whole maps, or the regions within `radius` around a point (the sender's position by default).
pub fn update(core: &Core, s: &Session, m: &Matched, sender: &CommandSender, say: Say) -> i32 {
    let strategy = match m.usage.split_whitespace().next() {
        Some("force-update") => TileUpdateStrategy::ForceAll,
        Some("fix-edges") => TileUpdateStrategy::ForceEdge,
        _ => TileUpdateStrategy::ForceNone,
    };
    let radius = m.int("radius");
    let center = match (m.int("x"), m.int("z"), sender.position) {
        (Some(x), Some(z), _) => Some((x, z)),
        (_, _, Some([x, _, z])) if radius.is_some() => Some((x.floor() as i32, z.floor() as i32)),
        _ => None,
    };
    if radius.is_some() && center.is_none() {
        say(text::one("You need to be in a world to use this command with a radius!", NEGATIVE));
        return 0;
    }
    let maps: Vec<String> = match m.get("map") {
        Some(map) => {
            if m.int("x").is_none()
                && radius.is_some()
                && s.map_server_world.get(map).cloned().flatten() != sender.world
            {
                say(text::one("The map does not belong to the same world you are currently in!", NEGATIVE));
                return 0;
            }
            vec![map.to_owned()]
        }
        None => {
            let mut ids: Vec<String> = s
                .map_server_world
                .iter()
                .filter(|(_, w)| w.is_some() && *w == &sender.world)
                .map(|(id, _)| id.clone())
                .collect();
            ids.sort();
            ids
        }
    };
    if maps.is_empty() {
        say(text::one("No map has been found for this world that could be updated!", NEGATIVE));
        return 0;
    }
    say(text::one("Creating update-tasks ...", BASE));
    let worlds: BTreeSet<String> = maps.iter().filter_map(|id| s.map_server_world.get(id).cloned().flatten()).collect();
    for world in worlds {
        core.save_world(Some(world));
    }
    for map in &maps {
        let regions = match (center, radius) {
            (Some((x, z)), Some(r)) => Regions::Only(regions_around(x, z, r)),
            _ => Regions::All,
        };
        s.queue.schedule(RenderTask { map: map.clone(), regions, strategy });
        say(text::lines(vec![text::format("Created new update-task for map %", &[map])]));
    }
    say(text::lines(vec![text::format("Use % to see the progress", &["/bluemap"])]));
    1
}

fn regions_around(x: i32, z: i32, radius: i32) -> BTreeSet<(i32, i32)> {
    let r = radius.abs();
    let range = |c: i32| (c.saturating_sub(r) >> REGION_SHIFT)..=(c.saturating_add(r) >> REGION_SHIFT);
    range(x).flat_map(|rx| range(z).map(move |rz| (rx, rz))).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn regions_cover_the_square() {
        assert_eq!(
            super::regions_around(0, 0, 10).into_iter().collect::<Vec<_>>(),
            [(-1, -1), (-1, 0), (0, -1), (0, 0)]
        );
        assert_eq!(super::regions_around(100, 100, 5).len(), 1);
    }
}
