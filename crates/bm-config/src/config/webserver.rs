use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::cidr::IpCidr;
use crate::de::fields;

/// `webserver.conf` (`WebserverConfig.java`); serializes with Java's field names.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, rename_all(serialize = "camelCase", deserialize = "kebab-case"))]
pub struct WebserverConfig {
    pub enabled: bool,
    pub webroot: PathBuf,
    /// Hidden key. `""`, `0.0.0.0` and `::0` mean all interfaces, `#getLocalHost` the host's own address.
    pub ip: String,
    pub port: i32,
    pub sse_enabled: bool,
    pub log: WebserverLogConfig,
    /// In file order. Java default is empty; the template writes two cache headers.
    #[serde(deserialize_with = "fields::string_map")]
    pub additional_headers: Vec<(String, String)>,
    /// Hidden bluemap-rs key: `ETag` on map data so reloads revalidate with 304s (Java sends none).
    pub map_etags: bool,
    /// Hidden bluemap-rs key: clients may unpack the hires tiles of an `optimized` storage themselves (docs/18).
    pub client_unpack: bool,
    /// bluemap-rs key: packed hires tiles for clients that don't unpack them.
    #[serde(deserialize_with = "server_unpack")]
    pub server_unpack: ServerUnpack,
    /// bluemap-rs key: peers whose `X-Forwarded-For`/`X-Real-IP` names the client for the `server-unpack` fair
    /// share (docs/18). Empty: the peer is the client.
    pub trusted_proxies: Vec<IpCidr>,
}

/// `server-unpack`: what a client that doesn't unpack gets for a packed hires tile (docs/18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServerUnpack {
    /// Unpacked on the server, a bounded number at once; `503` beyond that.
    #[default]
    Limited,
    /// `406`.
    Never,
    /// Unpacked on the server, bounded only by the transcode memory gate.
    Always,
}

fn server_unpack<'de, D: serde::Deserializer<'de>>(d: D) -> Result<ServerUnpack, D::Error> {
    match String::deserialize(d)?.to_ascii_lowercase().as_str() {
        "limited" => Ok(ServerUnpack::Limited),
        "never" => Ok(ServerUnpack::Never),
        "always" => Ok(ServerUnpack::Always),
        other => Err(serde::de::Error::custom(format!(
            "unknown server-unpack '{other}' (expected limited, never or always)"
        ))),
    }
}

/// `log { file, append, format }`; `file` and `format` are Java `String.format` patterns, passed through verbatim.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default, rename_all(serialize = "camelCase", deserialize = "kebab-case"))]
pub struct WebserverLogConfig {
    pub file: Option<String>,
    pub append: bool,
    pub format: String,
}

impl Default for WebserverConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            webroot: PathBuf::from("bluemap/web"),
            ip: "0.0.0.0".into(),
            port: 8100,
            sse_enabled: true,
            log: WebserverLogConfig::default(),
            additional_headers: Vec::new(),
            map_etags: true,
            client_unpack: true,
            server_unpack: ServerUnpack::default(),
            trusted_proxies: Vec::new(),
        }
    }
}

impl Default for WebserverLogConfig {
    fn default() -> Self {
        Self { file: None, append: false, format: r#"%1$s "%3$s %4$s %5$s" %6$s %7$s"#.into() }
    }
}
