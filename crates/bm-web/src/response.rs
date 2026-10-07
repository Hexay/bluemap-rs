//! Response helpers and `BlueMapResponseModifier`: `Server: BlueMap/<ver>`, then `additional-headers` only where
//! the handler set nothing (so live data keeps `no-store`).

use axum::body::Body;
use http::header::{CONTENT_LENGTH, HeaderName, SERVER, TRANSFER_ENCODING};
use http::{HeaderValue, Response, StatusCode};

use crate::WebError;

pub(crate) fn empty(status: StatusCode) -> Response<Body> {
    let mut res = Response::new(Body::empty());
    *res.status_mut() = status;
    res
}

pub(crate) fn header_static(res: &mut Response<Body>, name: HeaderName, value: &'static str) {
    res.headers_mut().insert(name, HeaderValue::from_static(value));
}

/// Validated `additional-headers`, in config order.
#[derive(Debug, Clone, Default)]
pub struct ExtraHeaders(Vec<(HeaderName, HeaderValue)>);

impl ExtraHeaders {
    /// Rejects names/values HTTP can't carry and framing headers, which Java overwrote anyway.
    pub fn new(pairs: &[(String, String)]) -> Result<Self, WebError> {
        let mut out = Vec::with_capacity(pairs.len());
        for (name, value) in pairs {
            let bad = |reason| WebError::Header { name: name.clone(), reason };
            let n = HeaderName::from_bytes(name.as_bytes()).map_err(|_| bad("not a valid header name"))?;
            if n == CONTENT_LENGTH || n == TRANSFER_ENCODING {
                return Err(bad("framing headers are set by the server"));
            }
            let v = HeaderValue::from_str(value).map_err(|_| bad("value contains characters HTTP can't carry"))?;
            out.push((n, v));
        }
        Ok(Self(out))
    }
}

pub(crate) fn finish(res: &mut Response<Body>, server: &HeaderValue, extra: &ExtraHeaders) {
    let headers = res.headers_mut();
    headers.insert(SERVER, server.clone());
    for (name, value) in &extra.0 {
        if !headers.contains_key(name) {
            headers.insert(name.clone(), value.clone());
        }
    }
}
