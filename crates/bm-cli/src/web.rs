//! `BlueMapCLI.startWebserver`: webroot + every configured map's storage, served until Ctrl+C.

use std::sync::Arc;

use anyhow::{Context, Result};
use bm_engine::Service;
use bm_web::{LiveMap, MapRoute, WebApp, WebOptions, WebServer};

use crate::log;

/// The running server; dropping it doesn't stop it, [`Webserver::wait`] blocks until Ctrl+C.
pub struct Webserver {
    runtime: tokio::runtime::Runtime,
    task: tokio::task::JoinHandle<Result<(), bm_web::WebError>>,
}

pub fn start(service: &Service, verbose: bool) -> Result<Webserver> {
    log::info("Starting webserver ...");
    let config = &service.config.webserver;
    std::fs::create_dir_all(&config.webroot).with_context(|| format!("create {}", config.webroot.display()))?;
    let mut app = WebApp::new(WebOptions::from_config(config, verbose)?)?;
    for (id, map) in &service.config.maps {
        // like Java: maps this instance renders get live markers, display-only maps are served from storage
        let live = map.world.is_some().then(|| {
            let live = LiveMap::new(config.sse_enabled).with_markers();
            live.set_markers("{}");
            Arc::new(live)
        });
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
    let task = runtime.spawn(server.serve(app, async {
        let _ = tokio::signal::ctrl_c().await;
    }));
    Ok(Webserver { runtime, task })
}

impl Webserver {
    pub fn wait(self) -> Result<()> {
        self.runtime.block_on(self.task).context("webserver task")??;
        Ok(())
    }
}
