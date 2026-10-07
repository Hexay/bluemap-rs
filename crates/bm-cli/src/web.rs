//! `BlueMapCLI.startWebserver`: webroot + every configured map's storage, served until shutdown. Maps loaded for
//! rendering in this run get Java's live routes: `live/markers.json` and, with `sse-enabled`, `live/sse` pushing
//! `tile` events as tiles are written.

use std::sync::Arc;

use anyhow::{Context, Result};
use bm_engine::{MapContext, Service};
use bm_web::{LiveMap, MapRoute, WebApp, WebOptions, WebServer};

use crate::log;
use crate::shutdown::Shutdown;

/// The running server; [`Webserver::wait`] blocks until it stopped after shutdown.
pub struct Webserver {
    runtime: tokio::runtime::Runtime,
    task: tokio::task::JoinHandle<Result<(), bm_web::WebError>>,
}

/// `loaded`: the maps this run renders; their tile listeners are hooked up to SSE here.
pub fn start(service: &Service, verbose: bool, loaded: &mut [MapContext], shutdown: &Shutdown) -> Result<Webserver> {
    log::info("Starting webserver ...");
    let config = &service.config.webserver;
    std::fs::create_dir_all(&config.webroot).with_context(|| format!("create {}", config.webroot.display()))?;
    let mut app = WebApp::new(WebOptions::from_config(config, verbose)?)?;
    for id in service.config.maps.keys() {
        let live = loaded.iter_mut().find(|m| &m.id == id).map(|map| live_map(map, config.sse_enabled));
        app.add_map(id.clone(), MapRoute { storage: service.map_storage(id)?, live })?;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().context("start web runtime")?;
    let server = runtime.block_on(WebServer::bind(&config.ip, config.port)).with_context(|| {
        format!(
            "BlueMap failed to bind to the configured address. This usually happens when the configured port ({}) \
             is already in use by some other program.",
            config.port
        )
    })?;
    let task = runtime.spawn(server.serve(app, shutdown.wait()));
    Ok(Webserver { runtime, task })
}

/// `MapRequestHandler(map, null, LiveMarkersDataSupplier, sse)`: live markers, no live players (the CLI has none).
fn live_map(map: &mut MapContext, sse: bool) -> Arc<LiveMap> {
    let live = Arc::new(LiveMap::new(sse).with_markers());
    live.set_markers(map.markers_json.clone());
    if sse {
        let events = live.clone();
        map.tile_listener = Some(Arc::new(move |(x, z), lod| events.tile_updated(x, z, lod)));
    }
    live
}

impl Webserver {
    pub fn wait(self) -> Result<()> {
        self.runtime.block_on(self.task).context("webserver task")??;
        Ok(())
    }
}
