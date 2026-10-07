//! The request pipeline: `LoggingRequestHandler` → `BlueMapResponseModifier` → `RoutingRequestHandler`
//! (`maps/<id>/(.*)` per map, last added wins, else the webroot).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use http::request::Parts;
use http::{HeaderValue, Response, StatusCode, Version};
use tokio_util::sync::CancellationToken;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;

use crate::WebError;
use crate::access_log::{AccessLog, FileSink, LogSink, RequestInfo, StdoutSink};
use crate::map_handler::{self, MapRoute};
use crate::paths::{decode_path, java_query_string};
use crate::response::{ExtraHeaders, finish};
use crate::static_files::StaticFiles;
use crate::webapp::WEBAPP_VERSION;

/// Requests never need a body; anything bigger is refused before it is read (#828).
const MAX_BODY: usize = 64 * 1024;
/// Upper bound for producing response headers (storage reads, transcoding); SSE bodies are not affected.
const HANDLER_TIMEOUT: Duration = Duration::from_secs(60);

/// Everything the handlers need besides the maps.
pub struct WebOptions {
    pub webroot: PathBuf,
    /// `webserver.conf` `additional-headers`, in file order.
    pub additional_headers: Vec<(String, String)>,
    /// `Server` header value.
    pub server_name: String,
    /// Serve the bundled webapp for files missing from `webroot`.
    pub serve_embedded_webapp: bool,
    pub access_log: AccessLog,
    /// Send `ETag` (and `Vary`) on map data. Off by default: Java sends neither, and the conformance suite holds
    /// us to its headers. A matching `If-None-Match` gets a 304 either way.
    pub map_etags: bool,
}

impl WebOptions {
    pub fn new(webroot: impl Into<PathBuf>) -> Self {
        Self {
            webroot: webroot.into(),
            additional_headers: Vec::new(),
            server_name: format!("BlueMap/{WEBAPP_VERSION}"),
            serve_embedded_webapp: true,
            access_log: AccessLog::disabled(),
            map_etags: false,
        }
    }

    /// From `webserver.conf`; `verbose` adds the console log like BlueMap CLI's `-b`.
    pub fn from_config(config: &bm_config::WebserverConfig, verbose: bool) -> Result<Self, WebError> {
        let mut sinks: Vec<Box<dyn LogSink>> = Vec::new();
        if verbose {
            sinks.push(Box::new(StdoutSink));
        }
        if let Some(file) = &config.log.file {
            sinks.push(Box::new(FileSink::open(file, config.log.append)?));
        }
        Ok(Self {
            additional_headers: config.additional_headers.clone(),
            access_log: AccessLog::new(&config.log.format, sinks)?,
            map_etags: config.map_etags,
            ..Self::new(&config.webroot)
        })
    }
}

/// Request-scoped peer address, inserted by the server per connection.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PeerAddr(pub SocketAddr);

pub struct WebApp {
    statics: StaticFiles,
    maps: Vec<(String, MapRoute)>,
    server: HeaderValue,
    extra: ExtraHeaders,
    log: AccessLog,
    map_etags: bool,
    shutdown: CancellationToken,
}

impl WebApp {
    pub fn new(options: WebOptions) -> Result<Self, WebError> {
        let server = HeaderValue::from_str(&options.server_name).map_err(|_| WebError::Header {
            name: "Server".into(),
            reason: "value contains characters HTTP can't carry",
        })?;
        Ok(Self {
            statics: StaticFiles::new(&options.webroot, options.serve_embedded_webapp),
            maps: Vec::new(),
            server,
            extra: ExtraHeaders::new(&options.additional_headers)?,
            log: options.access_log,
            map_etags: options.map_etags,
            shutdown: CancellationToken::new(),
        })
    }

    /// Serves `maps/<id>/…` from `route`; a later map with the same id replaces the earlier one.
    pub fn add_map(&mut self, id: impl Into<String>, route: MapRoute) -> Result<(), WebError> {
        let id = id.into();
        if id.is_empty() || id.contains('/') {
            return Err(WebError::MapId(id));
        }
        self.maps.push((id, route));
        Ok(())
    }

    /// Cancelled on server shutdown so open SSE streams end and graceful shutdown can finish.
    pub(crate) fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    /// Whether requests need a [`PeerAddr`] (only the access log reads it).
    pub(crate) fn wants_peer_addr(&self) -> bool {
        self.log.is_enabled()
    }

    pub fn into_router(self) -> Router {
        Router::new()
            .fallback(dispatch)
            .with_state(Arc::new(self))
            .layer(TimeoutLayer::with_status_code(StatusCode::SERVICE_UNAVAILABLE, HANDLER_TIMEOUT))
            .layer(RequestBodyLimitLayer::new(MAX_BODY))
            .layer(CatchPanicLayer::new())
    }

    async fn route(&self, req: &Parts, route_path: &str, query: &str) -> Response<Body> {
        for (id, map) in self.maps.iter().rev() {
            let rest = route_path
                .strip_prefix("maps/")
                .and_then(|r| r.strip_prefix(id.as_str()))
                .and_then(|r| r.strip_prefix('/'));
            // Java's `(.*)` doesn't cross line terminators, so such paths fall through to the webroot
            if let Some(rest) = rest.filter(|r| !r.contains(['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}'])) {
                let rest = rest.strip_prefix('/').unwrap_or(rest);
                let rest = if rest.is_empty() { "/" } else { rest };
                return map_handler::handle(map, rest, &req.method, &req.headers, self.map_etags, &self.shutdown)
                    .await;
            }
        }
        self.statics.handle(&req.method, route_path, query, &req.headers).await
    }
}

async fn dispatch(State(app): State<Arc<WebApp>>, req: Request) -> Response<Body> {
    // the body is never read; dropping it lets hyper discard it (bounded by the body limit layer)
    let (req, _) = req.into_parts();
    let path = decode_path(req.uri.path());
    let query = java_query_string(req.uri.query());
    let route_path = path.strip_prefix('/').unwrap_or(&path);
    let route_path = if route_path.is_empty() { "/" } else { route_path };
    let mut res = app.route(&req, route_path, &query).await;
    finish(&mut res, &app.server, &app.extra);
    if app.log.is_enabled() {
        let status = res.status();
        app.log.log(&request_info(&req, &path, &query), status.as_u16(), status.canonical_reason().unwrap_or(""));
    }
    res
}

fn request_info<'a>(req: &'a Parts, path: &'a str, query: &'a str) -> RequestInfo<'a> {
    let source = req.extensions.get::<PeerAddr>().map_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED), |p| p.0.ip());
    // Java keeps only the last header of a name and logs its first comma-separated value
    let forwarded_for = req
        .headers
        .get_all("x-forwarded-for")
        .iter()
        .next_back()
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let version = match req.version {
        Version::HTTP_09 => "HTTP/0.9",
        Version::HTTP_10 => "HTTP/1.0",
        Version::HTTP_2 => "HTTP/2.0",
        _ => "HTTP/1.1",
    };
    RequestInfo { source, forwarded_for, method: req.method.as_str(), path, query, version }
}
