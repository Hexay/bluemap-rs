//! `maps/<id>/…`: `MapRequestHandler` (live routes) in front of `MapStorageRequestHandler` (tiles and items).

use std::sync::Arc;

use axum::body::Body;
use bm_storage::{GridKey, ItemKey, MapStorage, Stored};
use http::header::{CONTENT_ENCODING, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, Response, StatusCode};
use tokio_util::sync::CancellationToken;

use crate::content_type;
use crate::encoding::{Accepted, encode};
use crate::live::LiveMap;
use crate::response::{empty, header_static};

/// One served map: its storage and, when the app provides live data, its [`LiveMap`].
#[derive(Clone)]
pub struct MapRoute {
    pub storage: Arc<dyn MapStorage>,
    pub live: Option<Arc<LiveMap>>,
}

const NO_STORE: [&str; 4] = ["Cache-Control", "Cloudflare-CDN-Cache-Control", "CDN-Cache-Control", "Surrogate-Control"];

pub(crate) async fn handle(
    route: &MapRoute,
    path: &str,
    headers: &HeaderMap,
    shutdown: &CancellationToken,
) -> Response<Body> {
    if let Some(live) = &route.live {
        match path {
            "live/players.json" if live.players().is_some() => return live_json(live.players().flatten()),
            "live/markers.json" if live.markers().is_some() => return live_json(live.markers().flatten()),
            "live/sse" if live.sse_enabled() => return sse(live, shutdown.clone()),
            _ => {}
        }
    }
    storage(route.storage.clone(), path, Accepted::from_headers(headers)).await
}

fn live_json(body: Option<bytes::Bytes>) -> Response<Body> {
    let mut res = Response::new(body.map_or_else(Body::empty, Body::from));
    header_static(&mut res, CONTENT_TYPE, "application/json");
    no_store(&mut res);
    res
}

fn sse(live: &Arc<LiveMap>, shutdown: CancellationToken) -> Response<Body> {
    let Some(stream) = live.subscribe(shutdown) else { return empty(StatusCode::NOT_FOUND) };
    let mut res = Response::new(Body::from_stream(stream));
    header_static(&mut res, CONTENT_TYPE, "text/event-stream");
    no_store(&mut res);
    res.headers_mut().insert("X-Accel-Buffering", HeaderValue::from_static("no"));
    res
}

fn no_store(res: &mut Response<Body>) {
    for name in NO_STORE {
        res.headers_mut().insert(name, HeaderValue::from_static("no-store"));
    }
}

enum Target {
    Tile(GridKey, i32, i32),
    Item(ItemKey),
}

async fn storage(storage: Arc<dyn MapStorage>, path: &str, accepted: Accepted) -> Response<Body> {
    let path = path.strip_prefix('/').unwrap_or(path);
    let path = path.strip_suffix('/').unwrap_or(path);
    let (path, gz_url) = match path.strip_suffix(".gz") {
        Some(p) => (p, true),
        None => (path, false),
    };
    let target = match parse_tile(path) {
        Some(Ok((lod, x, z))) => Target::Tile(if lod == 0 { GridKey::Hires } else { GridKey::Lowres(lod) }, x, z),
        Some(Err(())) => return empty(StatusCode::NOT_FOUND),
        None => match item_key(path) {
            Some(item) => Target::Item(item),
            None => return empty(StatusCode::NOT_FOUND),
        },
    };
    let content_type = match &target {
        Target::Tile(GridKey::Hires, ..) => content_type::OCTET_STREAM,
        Target::Tile(..) => "image/png",
        Target::Item(_) => content_type::map_item(path),
    };
    let is_png = content_type == "image/png";
    let is_tile = matches!(target, Target::Tile(..));
    let read = tokio::task::spawn_blocking(move || -> Result<Option<_>, String> {
        let stored: Option<Stored> = match target {
            Target::Tile(grid, x, z) => storage.read_grid(grid, (x, z)),
            Target::Item(item) => storage.read_item(&item),
        }
        .map_err(|e| e.to_string())?;
        stored.map(|s| encode(s, is_png, gz_url, &accepted).map_err(|e| e.to_string())).transpose()
    })
    .await;
    match read {
        Ok(Ok(Some(encoded))) => {
            let mut res = Response::new(Body::from(encoded.body));
            header_static(&mut res, CONTENT_TYPE, content_type);
            if let Some(coding) = encoded.content_encoding {
                header_static(&mut res, CONTENT_ENCODING, coding);
            }
            res
        }
        Ok(Ok(None)) if is_tile => empty(StatusCode::NO_CONTENT),
        Ok(Ok(None)) => empty(StatusCode::NOT_FOUND),
        Ok(Err(e)) => {
            tracing::error!("Failed to read map-tile for web-request: {e}");
            empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
        Err(e) => {
            tracing::error!("map storage read panicked: {e}");
            empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

fn item_key(path: &str) -> Option<ItemKey> {
    Some(match path {
        "settings.json" => ItemKey::Settings,
        "textures.json" => ItemKey::Textures,
        "live/markers.json" => ItemKey::Markers,
        "live/players.json" => ItemKey::Players,
        _ => ItemKey::asset(path.strip_prefix("assets/")?),
    })
}

/// `tiles/([\d/]+)/x(-?[\d/]+)z(-?[\d/]+).*` (full match). `None` = no match; `Some(Err)` = matched but the
/// numbers don't parse (Java's `NumberFormatException` → 404).
pub(crate) fn parse_tile(path: &str) -> Option<Result<(u32, i32, i32), ()>> {
    let rest = path.strip_prefix("tiles/")?;
    // 'x' is outside [\d/], so group 1 is the whole digit/slash run minus the '/' before 'x'
    let (run, rest) = split_run(rest, 0);
    let lod = run.strip_suffix('/').filter(|l| !l.is_empty())?;
    let (x, rest) = split_run(rest.strip_prefix('x')?, 1);
    let (z, tail) = split_run(rest.strip_prefix('z')?, 1);
    let line_break = ['\n', '\r', '\u{85}', '\u{2028}', '\u{2029}'];
    if x.trim_start_matches('-').is_empty() || z.trim_start_matches('-').is_empty() || tail.contains(line_break) {
        return None;
    }
    let int = |s: &str| s.replace('/', "").parse::<i32>().map_err(|_| ());
    Some(lod.parse::<i32>().map_err(|_| ()).and_then(|lod| Ok((lod.unsigned_abs(), int(x)?, int(z)?))))
}

/// Splits off `-?[\d/]*` (the sign only when `signed` is 1).
fn split_run(s: &str, signed: usize) -> (&str, &str) {
    let start = if s.starts_with('-') { signed } else { 0 };
    let run = s[start..].len() - s[start..].trim_start_matches(|c: char| c.is_ascii_digit() || c == '/').len();
    s.split_at(start + run)
}
