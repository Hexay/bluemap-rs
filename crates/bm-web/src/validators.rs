//! Map-data validators (Java sends none: docs/11 #1). The ETag is the storage [`Version`] plus the body's coding,
//! so each representation (identity, stored coding, transcoded gzip) has its own strong tag, like the `-gzip`
//! suffix on static files; quoted, unlike Java's static tags.

use bm_storage::Version;
use http::header::IF_NONE_MATCH;
use http::{HeaderMap, HeaderValue, Method};

pub(crate) fn etag(version: Version, coding: Option<&str>) -> HeaderValue {
    let tag = match coding {
        Some(c) => format!("\"{version}-{c}\""),
        None => format!("\"{version}\""),
    };
    HeaderValue::from_str(&tag).expect("hex, '-' and coding ids are valid header text")
}

/// The `If-None-Match` of a GET or HEAD (other methods never get a 304).
#[derive(Debug, Clone, Default)]
pub(crate) struct IfNoneMatch(Vec<HeaderValue>);

impl IfNoneMatch {
    pub fn from_request(method: &Method, headers: &HeaderMap) -> Option<Self> {
        let values: Vec<_> = headers.get_all(IF_NONE_MATCH).iter().cloned().collect();
        (matches!(*method, Method::GET | Method::HEAD) && !values.is_empty()).then_some(Self(values))
    }

    /// Weak comparison (RFC 9110 §13.1.2): `W/` is ignored; `*` matches any existing representation.
    pub fn matches(&self, etag: &HeaderValue) -> bool {
        let etag = etag.as_bytes();
        self.0.iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(',')).any(|t| {
            let t = t.trim();
            t == "*" || t.strip_prefix("W/").unwrap_or(t).as_bytes() == etag
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inm(values: &[&str]) -> IfNoneMatch {
        let mut h = HeaderMap::new();
        for v in values {
            h.append(IF_NONE_MATCH, HeaderValue::from_str(v).unwrap());
        }
        IfNoneMatch::from_request(&Method::GET, &h).unwrap()
    }

    #[test]
    fn if_none_match_lists_weak_tags_and_star() {
        let tag = HeaderValue::from_static("\"1-2-3-gzip\"");
        assert!(inm(&["\"1-2-3-gzip\""]).matches(&tag));
        assert!(inm(&["\"x\", W/\"1-2-3-gzip\""]).matches(&tag));
        assert!(inm(&["\"x\"", "\"1-2-3-gzip\""]).matches(&tag));
        assert!(inm(&["*"]).matches(&tag));
        assert!(!inm(&["\"1-2-3\""]).matches(&tag), "identity tag must not match the gzip representation");
        assert!(!inm(&["1-2-3-gzip"]).matches(&tag));
        let h: HeaderMap = [(IF_NONE_MATCH, HeaderValue::from_static("*"))].into_iter().collect();
        assert!(IfNoneMatch::from_request(&Method::POST, &h).is_none());
    }
}
