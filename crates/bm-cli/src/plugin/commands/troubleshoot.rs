//! `TroubleshootCommand`: runs the checks in upstream's order and reports the first failure.

use std::sync::{Arc, PoisonError};

use bm_engine::MapContext;
use bm_ipc::CommandSender;
use serde_json::Value;

use super::super::core::Core;
use super::super::session::Session;
use super::super::text::{self, POSITIVE, WARNING};
use super::Say;
use super::checks::{self, Check, Lines};
use super::parse::Matched;

pub fn run(core: &Core, s: &Session, m: &Matched, sender: &CommandSender) -> Lines {
    let world = sender_world(core, sender);
    // `position.toVector2(true).toInt()` truncates towards zero
    let sender_pos = sender.position.map(|[x, _, z]| (x as i32, z as i32));
    let result = match (m.get("map").and_then(|id| s.maps.get(id)), m.int("x"), m.int("z")) {
        (Some(map), Some(x), Some(z)) => troubleshoot(s, &[map], Some((x, z))),
        (Some(map), _, _) => world
            .as_ref()
            .map_or(Ok(()), |w| checks::map_has_correct_world(s, &map, w))
            .and_then(|()| troubleshoot(s, &[map], sender_pos)),
        _ => {
            let maps = match &world {
                Some(w) => {
                    let mut maps: Vec<_> = s
                        .maps
                        .all()
                        .into_iter()
                        .filter(|m| s.map_server_world.get(&m.id).cloned().flatten().as_deref() == Some(&w.id))
                        .collect();
                    maps.sort_by_key(|m| m.config.sorting);
                    checks::world_has_maps(&maps, w).map(|()| maps)
                }
                None => Ok(s.maps.all()),
            };
            maps.and_then(|maps| troubleshoot(s, &maps, sender_pos))
        }
    };
    match result {
        Ok(()) => vec![vec![text::span("✔ no issues found", POSITIVE)]],
        Err(lines) => recolor(lines),
    }
}

pub fn command(core: &Core, s: &Session, m: &Matched, sender: &CommandSender, say: Say) -> i32 {
    say(text::paragraph("Troubleshooting", run(core, s, m, sender)));
    1
}

fn sender_world(core: &Core, sender: &CommandSender) -> Option<bm_ipc::WorldInfo> {
    let id = sender.world.as_ref()?;
    core.worlds.lock().unwrap_or_else(PoisonError::into_inner).iter().find(|w| &w.id == id).cloned()
}

fn recolor(lines: Lines) -> Lines {
    lines
        .into_iter()
        .map(|line| {
            line.into_iter()
                .map(|mut span: Value| {
                    if span["color"] == "" {
                        span["color"] = Value::from(WARNING);
                    }
                    span
                })
                .collect()
        })
        .collect()
}

fn troubleshoot(s: &Session, maps: &[Arc<MapContext>], pos: Option<(i32, i32)>) -> Check {
    checks::render_threads_running(s)?;
    if let Some(pos) = pos {
        for map in maps {
            tile_at(s, map, pos)?;
        }
        for map in maps {
            checks::tile_is_updated(s, map, pos)?;
        }
        for map in maps {
            checks::tile_inside_bounds(map, pos)?;
        }
    }
    for map in maps {
        checks::map_is_updated(s, map)?;
    }
    for map in maps {
        checks::map_is_not_frozen(s, map)?;
    }
    Ok(())
}

/// Tile problems only count once the tile is up to date and the map not frozen.
fn tile_at(s: &Session, map: &MapContext, pos: (i32, i32)) -> Check {
    let problems = checks::tile_problems(map, pos, checks::tile_info(map, pos));
    if problems.iter().all(Result::is_ok) {
        return Ok(());
    }
    checks::tile_is_updated(s, map, pos)?;
    checks::map_is_not_frozen(s, map)?;
    problems.into_iter().collect()
}
