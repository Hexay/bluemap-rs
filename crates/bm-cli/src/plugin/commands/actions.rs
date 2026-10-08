//! Commands that change state: reload, start/stop, freeze/unfreeze, purge, update, tasks cancel.

use std::collections::BTreeSet;
use std::sync::PoisonError;

use bm_engine::{Regions, RenderTask, TileUpdateStrategy};
use bm_ipc::CommandSender;

use super::super::core::Core;
use super::super::ops;
use super::super::session::Session;
use super::super::text::{self, BASE, FROZEN, INFO, NEGATIVE, POSITIVE};
use super::parse::Matched;
use super::{Say, info, task_ref};

/// Region files are 512×512 blocks.
const REGION_SHIFT: i32 = 9;

pub fn reload(core: &Core, light: bool, say: Say) -> i32 {
    say(text::one("Reloading BlueMap...", INFO));
    if core.reload(light) {
        say(text::one("BlueMap reloaded!", POSITIVE));
        1
    } else {
        say(text::one("Could not load BlueMap! See logs or console for details!", NEGATIVE));
        0
    }
}

/// `StartCommand` / `StopCommand`; a start that stays paused by a beyond-parity reason (docs/15) says why.
pub fn start_stop(core: &Core, s: &Session, start: bool, say: Say) -> i32 {
    let enabled = s.state.lock().unwrap_or_else(PoisonError::into_inner).render_threads_enabled;
    let template = if enabled == start { "% Render-Threads are already %" } else { "% Render-Threads are now %" };
    ops::set_render_threads(core, s, start);
    let (icon, word, color) = if start { ("⛏", "running", POSITIVE) } else { ("❌", "stopped", NEGATIVE) };
    say(text::lines(text::fill(template, &[(icon, color), (word, color)], BASE)));
    let still = info::pause_lines(core, s);
    if start && !still.is_empty() {
        let lines = [vec![text::span("...but they stay paused:", BASE)]].into_iter().chain(still).collect();
        say(text::lines(lines));
    }
    1
}

/// `FreezeCommand` / `UnfreezeCommand`: no "already" case upstream.
pub fn freeze(core: &Core, s: &Session, map: &str, frozen: bool, say: Say) -> i32 {
    ops::set_frozen(core, s, map, frozen);
    let (template, icon) = if frozen {
        (
            "% Map % is now % and will no longer automatically update\n\
             Any currently scheduled updates for this map have been cancelled",
            ("❄", FROZEN),
        )
    } else {
        ("% Map % is no longer % and will update automatically", ("⛏", INFO))
    };
    say(text::lines(text::fill(template, &[icon, text::hl(map), ("frozen", FROZEN)], BASE)));
    1
}

/// `PurgeCommand`'s messages; the purge itself runs before the command returns.
pub fn purge(s: &Session, map: &str, say: Say) -> i32 {
    let update = !s.state.lock().unwrap_or_else(PoisonError::into_inner).is_frozen(map);
    let mut lines = text::fill("Scheduled a new task to purge map %", &[text::hl(map)], POSITIVE);
    lines.push(text::format("Use % to see the progress", &["/bluemap"]));
    if update {
        lines.push(Vec::new());
        let freeze = format!("/bluemap freeze {map}");
        lines.extend(text::fill(
            "BlueMap will automatically start rendering the map again once the purge is done\n\
             If you don't want this, use % before purging",
            &[text::hl(&freeze)],
            BASE,
        ));
    }
    say(text::lines(lines));
    if let Err(e) = ops::purge(s, map) {
        crate::log::error(&format!("Failed to purge map '{map}': {e:#}"));
        let msg = "There was an error trying to purge %, see console for details.";
        say(text::lines(text::fill(msg, &[text::hl(map)], NEGATIVE)));
        return 0;
    }
    1
}

pub fn cancel_tasks(s: &Session, task: Option<&str>, say: Say) -> i32 {
    let removed = match task {
        None => s.queue.remove_where(|_| true),
        Some(r) => s.queue.remove_where(|t| task_ref(t) == r),
    };
    let (msg, ok) = match (task, removed) {
        (None, 0) => ("There are no scheduled tasks", false),
        (None, _) => ("All tasks cancelled", true),
        (Some(_), 0) => ("Task is not pending or already completed", false),
        (Some(_), _) => ("Task cancelled", true),
    };
    say(text::one(msg, if ok { POSITIVE } else { NEGATIVE }));
    i32::from(ok)
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
            ids.sort_by_key(|id| s.maps.get(id).map(|m| m.config.sorting));
            ids
        }
    };
    if maps.is_empty() {
        say(text::one("No map has been found for this world that could be updated!", NEGATIVE));
        return 0;
    }
    say(text::one("Creating update-tasks ...", INFO));
    let worlds: BTreeSet<String> = maps.iter().filter_map(|id| s.map_server_world.get(id).cloned().flatten()).collect();
    for world in worlds {
        core.save_world(Some(world));
    }
    let regions = match (center, radius) {
        (Some((x, z)), Some(r)) => Regions::Only(regions_around(x, z, r)),
        _ => Regions::All,
    };
    // `scheduleRenderTasksNext`: ahead of the queue, in map order
    for map in maps.iter().rev() {
        s.queue.schedule_next(RenderTask::new(map.clone(), regions.clone(), strategy));
    }
    for map in &maps {
        say(text::lines(text::fill("Created new update-task for map %", &[text::hl(map)], POSITIVE)));
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
