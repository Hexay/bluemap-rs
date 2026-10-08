//! `commands/checks/*`: each check is `Ok(())` or `Err(failure description)`; the text is upstream's, its
//! uncoloured parts take the caller's warning colour.

use std::sync::{Arc, PoisonError};

use bm_engine::{MapContext, PauseReason, Regions};
use bm_ipc::WorldInfo;
use bm_map::renderstate::{TileInfo, TileState};
use serde_json::Value;

use super::super::rstate;
use super::super::session::{Session, world_id};
use super::super::text::{self, Arg, BASE, INFO, hl};

pub type Lines = Vec<Vec<Value>>;
pub type Check = Result<(), Lines>;

const DISCORD: &str = "discord";
const REGION_SHIFT: i32 = 9;

fn fail(parts: Vec<Lines>) -> Check {
    let mut out = Vec::new();
    for (i, part) in parts.into_iter().enumerate() {
        if i > 0 {
            out.push(Vec::new());
        }
        out.extend(part);
    }
    Err(out)
}

fn plain(template: &str, args: &[Arg]) -> Lines {
    text::fill(template, args, "")
}

fn base(template: &str, args: &[Arg]) -> Lines {
    text::fill(template, args, BASE)
}

fn wait_for_update() -> Lines {
    base("wait until the map finished updating\nyou can use % to see the update progress", &[hl("/bluemap")])
}

/// `WorldHasMapsCheck` / `MapHasCorrectWorldCheck`: the `world:`/`dimension:` block for `world`.
fn world_config(world: &WorldInfo) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    let folder = bm_config::generate::format_path(std::path::Path::new(&world.folder), &cwd);
    match world.dimension_type.as_deref().filter(|t| *t != world.dimension) {
        None => format!("┌\n│ world: \"{folder}\"\n│ dimension: \"{}\"\n└", world.dimension),
        Some(t) => {
            format!("┌\n│ world: \"{folder}\"\n│ dimension: \"{}\"\n│ dimension-type: \"{t}\"\n└", world.dimension)
        }
    }
}

pub fn world_has_maps(maps: &[Arc<MapContext>], world: &WorldInfo) -> Check {
    if !maps.is_empty() {
        return Ok(());
    }
    let config = world_config(world);
    fail(vec![
        plain("⚠ there are no maps configured for\nyour current world", &[]),
        base(
            "to configure a map for your current world,\nmake sure to set\n%\nin the maps config file",
            &[(&config, INFO)],
        ),
    ])
}

pub fn map_has_correct_world(s: &Session, map: &MapContext, world: &WorldInfo) -> Check {
    if s.map_server_world.get(&map.id).cloned().flatten().as_deref() == Some(world.id.as_str()) {
        return Ok(());
    }
    let (current, config, file) = (world_id(map), world_config(world), format!("maps/{}.conf", map.id));
    let cwd = std::env::current_dir().unwrap_or_default();
    let expected =
        format!("{}#{}", bm_config::generate::format_path(std::path::Path::new(&world.folder), &cwd), world.dimension);
    fail(vec![
        plain("⚠ map % is configured for\nworld % instead of\nworld %", &[hl(&map.id), hl(&current), hl(&expected)]),
        base(
            "to configure the map for your current world,\nmake sure to set\n%\nin the % config file",
            &[(&config, INFO), hl(&file)],
        ),
    ])
}

/// The first reason in [`PauseReason::ALL`] order, so a stopped queue reads as stopped.
pub fn render_threads_running(s: &Session) -> Check {
    let Some(reason) = s.queue.pause_reasons().iter().next() else { return Ok(()) };
    let paused = |why: &str, file: &str| {
        fail(vec![
            plain(&format!("⚠ render-threads are paused\n{why}"), &[]),
            base("this threshold can be configured in the %", &[hl(file)]),
        ])
    };
    match reason {
        PauseReason::PlayerLimit => paused("there are too many players online for rendering", "plugin.conf"),
        PauseReason::Memory => paused("the core uses more memory than its memory-limit", "core.conf"),
        PauseReason::ServerLoad => paused("the server is lagging", "plugin.conf"),
        PauseReason::Stopped => fail(vec![
            plain("⚠ render-threads are stopped", &[]),
            base("you can use % to start them", &[hl("/bluemap start")]),
        ]),
    }
}

pub fn map_is_updated(s: &Session, map: &MapContext) -> Check {
    let pending = s.queue.current_task().into_iter().chain(s.queue.pending_tasks()).any(|t| t.map == map.id);
    if !pending {
        return Ok(());
    }
    fail(vec![plain("⚠ map % has pending updates", &[hl(&map.id)]), wait_for_update()])
}

pub fn map_is_not_frozen(s: &Session, map: &MapContext) -> Check {
    if !s.state.lock().unwrap_or_else(PoisonError::into_inner).is_frozen(&map.id) {
        return Ok(());
    }
    let cmd = format!("/bluemap unfreeze {}", map.id);
    fail(vec![
        plain("⚠ map % is frozen\na frozen map will not be updated", &[hl(&map.id)]),
        base("you can use % to unfreeze the map", &[hl(&cmd)]),
    ])
}

/// `TileIsUpdatedCheck`: like upstream, nothing is pending while no task runs.
pub fn tile_is_updated(s: &Session, map: &MapContext, (x, z): (i32, i32)) -> Check {
    let Some(current) = s.queue.current_task() else { return Ok(()) };
    let region = (x >> REGION_SHIFT, z >> REGION_SHIFT);
    let covers = |t: &bm_engine::RenderTask| {
        t.map == map.id
            && match &t.regions {
                Regions::All => true,
                Regions::Only(set) => set.contains(&region),
            }
    };
    if !covers(&current) && !s.queue.pending_tasks().iter().any(covers) {
        return Ok(());
    }
    let (xs, zs) = (x.to_string(), z.to_string());
    fail(vec![
        plain("⚠ the region around (x:%, z:%) has pending\nupdates for map %", &[(&xs, ""), (&zs, ""), hl(&map.id)]),
        wait_for_update(),
    ])
}

pub fn tile_inside_bounds(map: &MapContext, (x, z): (i32, i32)) -> Check {
    if map.mask.is_column_inside(x, z) {
        return Ok(());
    }
    let file = format!("maps/{}.conf", map.id);
    fail(vec![
        plain("⚠ this position is outside the boundaries of map %", &[hl(&map.id)]),
        base(
            "if you expect this part of the map to be rendered\nmake sure your %\nin % is correct",
            &[hl("render-mask"), hl(&file)],
        ),
        base("more info about the % setting can\nbe found %", &[hl("render-mask"), hl("in the wiki")]),
    ])
}

/// The stored state of the hires tile at block `pos`.
pub fn tile_info(map: &MapContext, (x, z): (i32, i32)) -> TileInfo {
    rstate::tile_info(&map.storage, map.hires_grid.tile_of(x, z))
}

/// `TileNoRenderErrorCheck`, `TileNoChunkErrorCheck`, `TileHasLightDataCheck`, in that order.
pub fn tile_problems(map: &MapContext, pos: (i32, i32), info: TileInfo) -> Vec<Check> {
    let (xs, zs) = (pos.0.to_string(), pos.1.to_string());
    let at = [hl(&xs), hl(&zs), hl(&map.id)];
    let time = i64::from(info.render_time);
    let retry = || {
        base(
            "make sure your world is fully upgraded to your current\nminecraft-version and try loading this region \
             in-game\nwith a player\nif the problem persists, you can visit bluemaps % for help\nthe last time \
             bluemap tried to render this region\nwas % ago",
            &[hl(DISCORD), hl(&text::since(time))],
        )
    };
    let check = |state: TileState, lines: Vec<Lines>| if info.state == state { fail(lines) } else { Ok(()) };
    vec![
        check(
            TileState::RenderError,
            vec![
                plain("⚠ there was an error while rendering\naround (x:%, z:%) for map %", &at),
                base(
                    "check your server-logs for errors\naround %\nif the problem persists, you can visit bluemaps % for help",
                    &[hl(&text::date_time(time)), hl(DISCORD)],
                ),
            ],
        ),
        check(
            TileState::ChunkError,
            vec![plain("⚠ there was an error while loading a chunk\naround (x:%, z:%) for map %", &at), retry()],
        ),
        check(
            TileState::MissingLight,
            vec![plain("⚠ chunks are missing light-data\naround (x:%, z:%) for map %", &at), retry()],
        ),
    ]
}
