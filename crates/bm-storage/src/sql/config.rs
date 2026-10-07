//! What a SQL storage connects to and how (upstream `SQLConfig` + `Dialect`).

use std::time::Duration;

use bm_compress::Compression;

use crate::format::Format;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// MySQL and MariaDB (`MySQLCommandSet`).
    MySql,
    Postgres,
    Sqlite,
}

impl Dialect {
    pub fn name(self) -> &'static str {
        match self {
            Self::MySql => "mysql",
            Self::Postgres => "postgresql",
            Self::Sqlite => "sqlite",
        }
    }
}

/// Pool size for `max-connections: -1` (Java: unbounded). Connections open lazily, so the pool only grows to the
/// number of threads using it at once (render threads + persistence + webserver).
pub const UNBOUNDED_CONNECTIONS: u32 = 64;

/// Default [`SqlConfig::statement_timeout`]. Java has none; every statement here is bounded work (single rows,
/// pages of 1000), so minutes mean a stuck connection or lock, which must surface as an error.
pub const STATEMENT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct SqlConfig {
    /// sqlx or JDBC URL, credentials in it or in `properties`.
    pub url: String,
    /// Configured `dialect:`; must agree with the URL's driver (it picks the statements, as in Java). `None`: from
    /// the URL.
    pub dialect: Option<Dialect>,
    /// JDBC `connection-properties`; `user` and `password` are used.
    pub properties: Vec<(String, String)>,
    /// Statements run on every new connection; `None`: the dialect's defaults (upstream `Dialect`).
    pub init_sql: Option<Vec<String>>,
    /// Must match `[a-z0-9_]{0,32}`; `sql.php` hardcodes the default.
    pub table_prefix: String,
    pub compression: Compression,
    /// Never creates tables or keys and refuses writes (webserver-only setups, #749).
    pub read_only: bool,
    pub max_connections: u32,
    pub statement_timeout: Duration,
    pub format: Format,
}

impl SqlConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            dialect: None,
            properties: Vec::new(),
            init_sql: None,
            table_prefix: "bluemap_".into(),
            compression: Compression::Gzip,
            read_only: false,
            max_connections: UNBOUNDED_CONNECTIONS,
            statement_timeout: STATEMENT_TIMEOUT,
            format: Format::Compat,
        }
    }
}
