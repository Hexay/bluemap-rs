//! Request-target handling the way BlueMap's `HttpRequestInputStream`/`HttpRequest` see it: `URI.getPath()`
//! (percent-decoded), the query parsed into a `LinkedHashMap` and re-encoded with `URLEncoder`, and webroot path
//! resolution with `Path.resolve` + `normalize` + `startsWith` semantics, hardened for Windows names.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

/// `URI.getPath()`: percent-decoded as UTF-8, malformed sequences become U+FFFD.
pub fn decode_path(raw: &str) -> Cow<'_, str> {
    percent_decode_str(raw).decode_utf8_lossy()
}

/// `HttpRequest.setRawQueryString` then `getRawQueryString`: what BlueMap logs and puts into redirects.
pub fn java_query_string(raw: Option<&str>) -> String {
    let mut params: Vec<(String, String)> = Vec::new();
    for param in raw.unwrap_or("").split('&').filter(|p| !p.is_empty()) {
        let (k, v) = param.split_once('=').unwrap_or((param, ""));
        let (k, v) = (url_decode(k), url_decode(v));
        match params.iter_mut().find(|(pk, _)| *pk == k) {
            Some(slot) => slot.1 = v,
            None => params.push((k, v)),
        }
    }
    params
        .iter()
        .map(|(k, v)| if v.is_empty() { k.clone() } else { format!("{}={}", url_encode(k), url_encode(v)) })
        .collect::<Vec<_>>()
        .join("&")
}

/// `URLDecoder.decode(s, UTF_8)`, lenient where Java throws.
fn url_decode(s: &str) -> String {
    percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().into_owned()
}

/// `URLEncoder.encode(s, UTF_8)`.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'*' | b'_' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// Inside the root: the normalized relative path (`/`-separated, `""` = the root itself).
    Inside(String),
    /// Escapes the root (`..` past it, absolute paths): Java answers 403.
    Outside,
    /// Not a valid file name on this OS (Java: `InvalidPathException` → 404).
    Invalid,
}

const SEPARATORS: &[char] = if cfg!(windows) { &['/', '\\'] } else { &['/'] };

/// `webRoot.resolve(rel).normalize().startsWith(webRoot)` without touching the filesystem.
pub fn resolve(rel: &str) -> Resolved {
    if rel.starts_with(SEPARATORS) || has_drive_prefix(rel) {
        return Resolved::Outside;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in rel.split(SEPARATORS) {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Resolved::Outside;
                }
            }
            name if !valid_name(name) => return Resolved::Invalid,
            name => parts.push(name),
        }
    }
    Resolved::Inside(parts.join("/"))
}

fn has_drive_prefix(rel: &str) -> bool {
    cfg!(windows) && rel.as_bytes().get(1) == Some(&b':')
}

#[cfg(windows)]
fn valid_name(name: &str) -> bool {
    const RESERVED: &[&str] = &["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    // Win32 silently strips trailing dots/spaces and maps device names anywhere: refuse both
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let device = RESERVED.contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    !device
        && !name.ends_with(['.', ' '])
        && !name.chars().any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
}

#[cfg(not(windows))]
fn valid_name(name: &str) -> bool {
    !name.contains('\0')
}

/// `rel` as Java's `Path.resolve` keeps it before `normalize`: empty segments dropped, `.`/`..` kept.
pub fn unnormalized(rel: &str) -> String {
    rel.split(SEPARATORS).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("/")
}

/// `root` joined with a `/`-separated relative path using the OS separator.
pub fn join(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    rel.split('/').filter(|s| !s.is_empty()).for_each(|s| p.push(s));
    p
}

/// `Path.hashCode()`: `WindowsPath` hashes the upper-cased string, `UnixPath` the bytes.
pub fn java_path_hash(path: &Path) -> i32 {
    let s = path.to_string_lossy();
    if cfg!(windows) {
        s.encode_utf16().fold(0i32, |h, c| {
            let up = char::from_u32(c.into()).map_or(c, |ch| ch.to_uppercase().next().map_or(c, |u| u as u16));
            h.wrapping_mul(31).wrapping_add(up.into())
        })
    } else {
        s.bytes().fold(0i32, |h, b| h.wrapping_mul(31).wrapping_add(b.into()))
    }
}
