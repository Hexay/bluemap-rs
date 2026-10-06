use std::path::PathBuf;

use serde::Deserialize;

use crate::de::fields;

/// `webserver.conf` (`WebserverConfig.java`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
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
}

/// `log { file, append, format }`; `file` and `format` are Java `String.format` patterns, passed through verbatim.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
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
        }
    }
}

impl Default for WebserverLogConfig {
    fn default() -> Self {
        Self { file: None, append: false, format: r#"%1$s "%3$s %4$s %5$s" %6$s %7$s"#.into() }
    }
}
