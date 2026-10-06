//! BlueMap's integrated webserver, wire-compatible with Java BlueMap's (docs/04-storage-web.md §3-5): the webroot
//! (bundled webapp behind it), `maps/<id>/…` from map storage, live JSON and SSE. Built on axum/hyper; intended
//! differences from Java are listed in the conformance test (`tests/conformance.rs`).
//!
//! ```no_run
//! # async fn run(storage: std::sync::Arc<dyn bm_storage::MapStorage>) -> Result<(), bm_web::WebError> {
//! use bm_web::{LiveMap, MapRoute, WebApp, WebOptions, WebServer};
//! let mut app = WebApp::new(WebOptions::new("bluemap/web"))?;
//! let live = std::sync::Arc::new(LiveMap::new(true).with_markers());
//! app.add_map("world", MapRoute { storage, live: Some(live.clone()) })?;
//! let server = WebServer::bind("0.0.0.0", 8100).await?;
//! live.tile_updated(3, -2, 0);
//! server.serve(app, async { let _ = tokio::signal::ctrl_c().await; }).await
//! # }
//! ```

mod access_log;
mod app;
mod content_type;
mod encoding;
mod error;
mod javafmt;
mod live;
mod map_handler;
mod paths;
mod response;
mod server;
mod static_files;
mod webapp;

pub use access_log::{AccessLog, FileSink, Level, LogSink, RequestInfo, StdoutSink, java_host_address};
pub use app::{WebApp, WebOptions};
pub use error::WebError;
pub use javafmt::{Arg, FormatError, JavaFormat};
pub use live::LiveMap;
pub use map_handler::MapRoute;
pub use server::WebServer;
pub use webapp::{WEBAPP_VERSION, install_webapp, webapp_files, write_settings};

#[cfg(test)]
mod tests;
