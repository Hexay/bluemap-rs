//! Live data for one map: the latest `players.json`/`markers.json` (pushed by the app, served with `no-store`) and
//! the `live/sse` event fan-out (`MapRequestHandler`, `SseConnection`). The app feeds it; the web layer only reads.

use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::Stream;
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_util::sync::CancellationToken;

/// `SseConnection.QUEUE_CAPACITY`: a client this far behind is disconnected.
const SSE_QUEUE: usize = 64;
const KEEPALIVE: Duration = Duration::from_secs(30);

pub struct LiveMap {
    sse: Option<broadcast::Sender<Bytes>>,
    players: Option<LiveJson>,
    markers: Option<LiveJson>,
    markers_read: Mutex<Option<Instant>>,
}

#[derive(Default)]
struct LiveJson(Mutex<Option<Bytes>>);

impl LiveJson {
    /// Stores `json`; true if it differs from the previous value (`LiveDataSupplierBroadcaster` equality).
    fn replace(&self, json: Bytes) -> bool {
        let mut latest = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if latest.as_ref() == Some(&json) {
            return false;
        }
        *latest = Some(json);
        true
    }

    fn get(&self) -> Option<Bytes> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl LiveMap {
    /// `sse`: serve `live/sse` (webserver.conf `sse-enabled`). Live JSON routes exist only once enabled with
    /// [`LiveMap::with_players`]/[`LiveMap::with_markers`]; otherwise those URLs fall through to map storage.
    pub fn new(sse: bool) -> Self {
        Self {
            sse: sse.then(|| broadcast::channel(SSE_QUEUE).0),
            players: None,
            markers: None,
            markers_read: Mutex::new(None),
        }
    }

    pub fn with_players(mut self) -> Self {
        self.players = Some(LiveJson::default());
        self
    }

    pub fn with_markers(mut self) -> Self {
        self.markers = Some(LiveJson::default());
        self
    }

    /// A rendered tile was saved; `lod` 0 = hires. The event names the tile z `y`, like Java.
    pub fn tile_updated(&self, x: i32, z: i32, lod: u32) {
        self.broadcast("tile", &format!("{{\"x\":{x},\"y\":{z},\"lod\":{lod}}}"));
    }

    /// New `players.json` body; broadcast as a `player` event when it changed.
    pub fn set_players(&self, json: impl Into<String>) {
        Self::update(self.players.as_ref(), json.into(), |data| self.broadcast("player", data));
    }

    /// New `markers.json` body; broadcast as a `marker` event when it changed.
    pub fn set_markers(&self, json: impl Into<String>) {
        Self::update(self.markers.as_ref(), json.into(), |data| self.broadcast("marker", data));
    }

    /// Connected SSE clients; the app may skip producing live data while this is 0, as Java does.
    pub fn sse_clients(&self) -> usize {
        self.sse.as_ref().map_or(0, broadcast::Sender::receiver_count)
    }

    /// Whether `live/markers.json` was served within `window` (marker demand of the plugin core).
    pub fn markers_read_within(&self, window: Duration) -> bool {
        self.markers_read.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t.elapsed() < window)
    }

    fn update(slot: Option<&LiveJson>, json: String, on_change: impl FnOnce(&str)) {
        if let Some(slot) = slot
            && slot.replace(Bytes::from(json.clone()))
        {
            on_change(&json);
        }
    }

    fn broadcast(&self, kind: &str, data: &str) {
        if let Some(tx) = &self.sse
            && tx.receiver_count() > 0
        {
            let _ = tx.send(encode_event(kind, data));
        }
    }

    pub(crate) fn sse_enabled(&self) -> bool {
        self.sse.is_some()
    }

    /// `None` = route not registered; `Some(None)` = registered but nothing pushed yet (Java: empty 200).
    pub(crate) fn players(&self) -> Option<Option<Bytes>> {
        self.players.as_ref().map(LiveJson::get)
    }

    pub(crate) fn markers(&self) -> Option<Option<Bytes>> {
        let markers = self.markers.as_ref()?;
        *self.markers_read.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
        Some(markers.get())
    }

    /// The event stream for one client; ends when it lags [`SSE_QUEUE`] events behind or on `shutdown`.
    pub(crate) fn subscribe(
        self: &Arc<Self>,
        shutdown: CancellationToken,
    ) -> Option<impl Stream<Item = Result<Bytes, Infallible>> + Send + 'static> {
        let rx = self.sse.as_ref()?.subscribe();
        Some(futures_util::stream::unfold((rx, shutdown), |(mut rx, shutdown)| async move {
            let next = tokio::select! {
                () = shutdown.cancelled() => return None,
                r = tokio::time::timeout(KEEPALIVE, rx.recv()) => r,
            };
            let chunk = match next {
                Err(_) => Bytes::from_static(b":\n"),
                Ok(Ok(event)) => event,
                Ok(Err(RecvError::Lagged(_) | RecvError::Closed)) => return None,
            };
            Some((Ok(chunk), (rx, shutdown)))
        }))
    }
}

/// `SseConnection.send`: `event:` line, one `data:` line per `String.lines()` line, blank line.
pub(crate) fn encode_event(kind: &str, data: &str) -> Bytes {
    let mut out = format!("event: {kind}\n");
    for line in java_lines(data) {
        out.push_str("data: ");
        out.push_str(line);
        out.push('\n');
    }
    out.push('\n');
    Bytes::from(out)
}

/// `String.lines()`: splits on `\n`, `\r`, `\r\n`; no trailing empty line.
fn java_lines(s: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        match rest.find(['\n', '\r']) {
            Some(i) => {
                lines.push(&rest[..i]);
                let skip = if rest[i..].starts_with("\r\n") { 2 } else { 1 };
                rest = &rest[i + skip..];
            }
            None => {
                lines.push(rest);
                break;
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_match_java_framing() {
        assert_eq!(encode_event("tile", "{\"x\":1}"), Bytes::from("event: tile\ndata: {\"x\":1}\n\n"));
        assert_eq!(encode_event("marker", "a\r\nb\rc\n"), Bytes::from("event: marker\ndata: a\ndata: b\ndata: c\n\n"));
        assert_eq!(encode_event("player", ""), Bytes::from("event: player\n\n"));
    }

    #[test]
    fn unchanged_live_json_is_not_rebroadcast() {
        let live = Arc::new(LiveMap::new(true).with_players());
        let mut rx = live.sse.as_ref().unwrap().subscribe();
        live.set_players("{}");
        live.set_players("{}");
        live.tile_updated(-3, 7, 0);
        assert_eq!(rx.try_recv().unwrap(), Bytes::from("event: player\ndata: {}\n\n"));
        assert_eq!(rx.try_recv().unwrap(), Bytes::from("event: tile\ndata: {\"x\":-3,\"y\":7,\"lod\":0}\n\n"));
        assert!(rx.try_recv().is_err());
        assert_eq!(live.players(), Some(Some(Bytes::from("{}"))));
        assert_eq!(live.markers(), None);
    }
}
