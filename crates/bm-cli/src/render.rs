//! `BlueMapCLI.renderMaps` without watching: webapp settings, resources, then every selected map once.

use std::time::{Duration, Instant};

use anyhow::Result;
use bm_engine::{Service, TileUpdateStrategy, UpdateEvent, UpdateStats, update_map};

use crate::log;

/// BlueMap's CLI reports progress every 10 s.
const PROGRESS_INTERVAL: Duration = Duration::from_secs(10);

/// Returns whether every map updated without errors.
pub fn render_maps(
    service: &Service,
    strategy: TileUpdateStrategy,
    maps: Option<&str>,
    force_webapp: bool,
) -> Result<bool> {
    if service.config.webapp.enabled {
        // `createOrUpdateWebApp`: webapp files when forced (-g) or missing, then settings.json
        bm_web::install_webapp(&service.config.webapp.webroot, force_webapp)?;
        service.write_webapp_settings()?;
    }
    service.resources()?;

    let selected: Option<Vec<&str>> = maps.map(|m| m.split(',').collect());
    let ids = service.map_ids(|id| selected.as_ref().is_none_or(|s| s.contains(&id)));
    let mut loaded = Vec::new();
    for id in &ids {
        match service.open_map(id) {
            Ok(Some(map)) => {
                log::info(&format!("Loading map '{id}'..."));
                map.warnings.iter().for_each(|w| log::warn(w));
                loaded.push(map);
            }
            Ok(None) => log::info(&format!(
                "The map '{id}' has no world configured. The map will be displayed, but it will not be updated by \
                 this bluemap instance!"
            )),
            Err(e) => log::error(&format!("Failed to load map '{id}': {e}")),
        }
    }

    log::info(&format!("Start updating {} maps ({} threads) ...", loaded.len(), rayon::current_num_threads()));
    let start = Instant::now();
    let mut ok = true;
    let mut total = UpdateStats::default();
    for map in &loaded {
        let mut last = Instant::now();
        let mut on_event = |event: UpdateEvent| match event {
            UpdateEvent::Warning(w) => log::warn(&w),
            UpdateEvent::Progress(s) if last.elapsed() >= PROGRESS_INTERVAL => {
                last = Instant::now();
                let pct = (s.regions_done as f64 / s.regions.max(1) as f64 * 100_000.0).round() / 1000.0;
                log::info(&format!("updating map '{}': {pct}%", map.id));
            }
            UpdateEvent::Progress(_) => {}
        };
        match update_map(map, service.resources()?, strategy, &mut on_event) {
            Ok(s) => {
                log::info(&format!(
                    "Map '{}': {} regions, {} tiles rendered, {} skipped, {} deleted, {} lowres tiles saved",
                    map.id, s.regions, s.tiles_rendered, s.tiles_skipped, s.tiles_deleted, s.lowres_saves
                ));
                ok &= s.tile_errors == 0;
                add(&mut total, &s);
            }
            Err(e) => {
                ok = false;
                log::error(&format!("Failed to update map '{}': {e}", map.id));
            }
        }
    }
    log::info(&format!(
        "Your maps are now all up-to-date! ({} tiles rendered in {:.1}s)",
        total.tiles_rendered,
        start.elapsed().as_secs_f64()
    ));
    Ok(ok)
}

fn add(total: &mut UpdateStats, s: &UpdateStats) {
    total.regions += s.regions;
    total.tiles_rendered += s.tiles_rendered;
    total.tiles_skipped += s.tiles_skipped;
    total.tiles_deleted += s.tiles_deleted;
    total.tile_errors += s.tile_errors;
    total.lowres_saves += s.lowres_saves;
}
