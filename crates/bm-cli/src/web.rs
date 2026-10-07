//! `BlueMapCLI.startWebserver`: webroot + every configured map's storage, served until shutdown. Maps loaded for
//! rendering in this run get Java's live routes: `live/markers.json` and, with `sse-enabled`, `live/sse` pushing
//! `tile` events as tiles are written. Plugin mode adds `live/players.json` through its own [`LiveMap`]s.

use std::collections::BTreeMap;
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

/// CLI: `loaded` are the maps this run renders; their tile listeners are hooked up to SSE here.
pub fn start(service: &Service, verbose: bool, loaded: &mut [MapContext], shutdown: &Shutdown) -> Result<Webserver> {
    let sse = service.config.webserver.sse_enabled;
    let lives: BTreeMap<String, Arc<LiveMap>> = loaded
        .iter_mut()
        .map(|map| {
            let live = Arc::new(LiveMap::new(sse).with_markers());
            live.set_markers(map.markers_json.clone());
            hook_tiles(map, &live, sse);
            (map.id.clone(), live)
        })
        .collect();
    serve(service, verbose, &lives, None, shutdown.wait())
}

/// Sends `tile` SSE events for every tile `map` writes.
pub fn hook_tiles(map: &mut MapContext, live: &Arc<LiveMap>, sse: bool) {
    if sse {
        let events = live.clone();
        map.tile_listener = Some(Arc::new(move |(x, z), lod| events.tile_updated(x, z, lod)));
    }
}

/// Serves the webroot and every configured map, with the live routes of `lives`, until `shutdown` resolves.
/// `workers`: tokio worker threads (default: one per core).
pub fn serve(
    service: &Service,
    verbose: bool,
    lives: &BTreeMap<String, Arc<LiveMap>>,
    workers: Option<usize>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<Webserver> {
    log::info("Starting webserver ...");
    let config = &service.config.webserver;
    std::fs::create_dir_all(&config.webroot).with_context(|| format!("create {}", config.webroot.display()))?;
    let mut app = WebApp::new(WebOptions::from_config(config, verbose)?)?;
    for id in service.config.maps.keys() {
        app.add_map(id.clone(), MapRoute { storage: service.map_storage(id)?, live: lives.get(id).cloned() })?;
    }
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    if let Some(n) = workers {
        builder.worker_threads(n);
    }
    let runtime = builder.thread_name("bluemap-web").enable_all().build().context("start web runtime")?;
    let server = runtime.block_on(WebServer::bind(&config.ip, config.port)).with_context(|| {
        format!(
            "BlueMap failed to bind to the configured address. This usually happens when the configured port ({}) \
             is already in use by some other program.",
            config.port
        )
    })?;
    let task = runtime.spawn(server.serve(app, shutdown));
    Ok(Webserver { runtime, task })
}

impl Webserver {
    pub fn wait(self) -> Result<()> {
        self.runtime.block_on(self.task).context("webserver task")??;
        Ok(())
    }
}
