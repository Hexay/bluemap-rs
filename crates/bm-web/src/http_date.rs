//! HTTP dates the way BlueMap's `FileRequestHandler` writes and reads them.

use chrono::{DateTime, Utc};

/// `DateTimeFormatter.RFC_1123_DATE_TIME`: the day of month is not zero-padded (`Tue, 6 Oct 2026 …`).
pub(crate) fn java_http_date(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .map_or_else(String::new, |t| t.format("%a, %-d %b %Y %H:%M:%S GMT").to_string())
}

pub(crate) fn parse_http_date(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc2822(s.trim()).ok().map(|t| t.timestamp_millis())
}
