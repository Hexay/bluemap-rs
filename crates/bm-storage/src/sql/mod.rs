//! SQL storage (MySQL/MariaDB, PostgreSQL, SQLite) with upstream's schema, keys and blob contents, so existing
//! databases and `sql.php` keep working. Blocking API over sqlx: calls `Handle::block_on`, so call it from plain
//! threads (render workers, `spawn_blocking`), never from inside an async task; the runtime must be multi-thread.

mod db;
mod hires;
mod keys;
mod map;
mod schema;
mod statements;
mod url;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use bm_compress::Compression;
use tokio::runtime::Handle;

use self::db::{Arg, Pool};
use self::hires::SqlHires;
use self::keys::{KeyCache, KeyTable};
pub use self::map::SqlMapStorage;
use self::statements::Statements;
use crate::api::{MapStorage, Storage};
use crate::error::{Error, Result};
use crate::format::{self, Format, SQL_MARKER};
use crate::optimized::OptimizedMapStorage;

const PAGE: i64 = 1000;
/// Statement bytes around the blob in one write packet (SQL text, ids, framing).
const PACKET_OVERHEAD: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// MySQL and MariaDB (`MySQLCommandSet`).
    MySql,
    Postgres,
    Sqlite,
}

#[derive(Debug, Clone)]
pub struct SqlConfig {
    /// sqlx or JDBC URL, credentials in it or in `properties`.
    pub url: String,
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
    pub format: Format,
}

impl SqlConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            properties: Vec::new(),
            init_sql: None,
            table_prefix: "bluemap_".into(),
            compression: Compression::Gzip,
            read_only: false,
            max_connections: 8,
            format: Format::Compat,
        }
    }
}

pub(crate) struct Shared {
    runtime: Handle,
    pool: Pool,
    sql: Statements,
    keys: KeyCache,
    compression: Compression,
    read_only: bool,
    /// MySQL `max_allowed_packet`; writes above it would fail mid-protocol (#694).
    max_packet: Option<usize>,
}

impl Shared {
    fn run<T>(&self, fut: impl Future<Output = T>) -> T {
        self.runtime.block_on(fut)
    }

    async fn key_id(&self, table: KeyTable, key: &str, create: bool) -> Result<Option<i64>> {
        self.keys.id(&self.pool, &self.sql, table, key, create && !self.read_only).await
    }

    fn check_writable(&self, blob_len: usize) -> Result<()> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        match self.max_packet {
            Some(limit) if blob_len + PACKET_OVERHEAD > limit => Err(Error::BlobTooLarge { size: blob_len, limit }),
            _ => Ok(()),
        }
    }
}

pub struct SqlStorage {
    shared: Arc<Shared>,
    format: Format,
    maps: Mutex<HashMap<String, Arc<SqlMapStorage>>>,
    optimized: Mutex<HashMap<String, Arc<OptimizedMapStorage>>>,
}

impl SqlStorage {
    /// Connects, makes sure the six tables exist (creating them unless read-only) and checks the stored format
    /// against `config.format` ([`crate::format`]).
    pub fn connect(config: &SqlConfig, runtime: Handle) -> Result<Self> {
        let storage = Self::connect_unchecked(config, runtime)?;
        let found = match storage.detect_format() {
            Ok(found) => found,
            Err(e) => {
                storage.close();
                return Err(e);
            }
        };
        let checked = format::check(&format!("{} (tables {}*)", redact(&config.url), config.table_prefix), config.format, found)
            .and_then(|()| match (config.format, found) {
                (Format::Optimized, None) if !config.read_only => storage.set_marker(true),
                _ => Ok(()),
            });
        if let Err(e) = checked {
            storage.close();
            return Err(e);
        }
        Ok(storage)
    }

    pub fn format(&self) -> Format {
        self.format
    }

    /// `Some(Optimized)` with the marker row, `Some(Compat)` with any map, `None` when empty.
    pub(crate) fn detect_format(&self) -> Result<Option<Format>> {
        let s = &self.shared;
        s.run(async {
            if s.key_id(KeyTable::GridStorage, SQL_MARKER, false).await?.is_some() {
                return Ok(Some(Format::Optimized));
            }
            let any = s.pool.fetch_texts(&s.sql.list_map_ids, &[Arg::Int(1), Arg::Int(0)]).await?;
            Ok((!any.is_empty()).then_some(Format::Compat))
        })
    }

    pub(crate) fn set_marker(&self, optimized: bool) -> Result<()> {
        let s = &self.shared;
        s.check_writable(0)?;
        s.run(async {
            if optimized {
                s.key_id(KeyTable::GridStorage, SQL_MARKER, true).await?;
            } else {
                s.pool.execute(&s.sql.delete_grid_storage, &[Arg::Str(SQL_MARKER)]).await?;
                s.keys.invalidate(KeyTable::GridStorage, SQL_MARKER);
            }
            Ok(())
        })
    }

    /// The map as an optimized map storage over these tables, whatever this storage's format (for conversion).
    pub(crate) fn optimized_map(&self, map_id: &str) -> Arc<OptimizedMapStorage> {
        let inner = self.sql_map(map_id);
        let mut maps = self.optimized.lock().unwrap_or_else(PoisonError::into_inner);
        maps.entry(map_id.to_owned())
            .or_insert_with(|| Arc::new(OptimizedMapStorage::new(inner.clone(), Box::new(SqlHires(inner)))))
            .clone()
    }

    pub(crate) fn connect_unchecked(config: &SqlConfig, runtime: Handle) -> Result<Self> {
        if !valid_prefix(&config.table_prefix) {
            return Err(Error::InvalidTablePrefix(config.table_prefix.clone()));
        }
        let (dialect, url) = url::connect_url(&config.url, &config.properties)?;
        let prefix = config.table_prefix.as_str();
        let (pool, max_packet) = runtime.block_on(async {
            let pool = Pool::connect(dialect, &url, config).await?;
            let ready = async {
                initialize_tables(&pool, dialect, prefix, config.read_only).await?;
                if dialect != Dialect::MySql {
                    return Ok(None);
                }
                let limit = pool.fetch_int("SELECT @@max_allowed_packet", &[]).await?;
                Ok::<_, Error>(limit.and_then(|l| usize::try_from(l).ok()))
            };
            match ready.await {
                Ok(max_packet) => Ok((pool, max_packet)),
                Err(e) => {
                    pool.close().await;
                    Err(e)
                }
            }
        })?;
        let shared = Shared {
            runtime,
            pool,
            sql: Statements::new(dialect, prefix),
            keys: KeyCache::default(),
            compression: config.compression,
            read_only: config.read_only,
            max_packet,
        };
        Ok(Self { shared: Arc::new(shared), format: config.format, maps: Mutex::default(), optimized: Mutex::default() })
    }

    pub fn sql_map(&self, map_id: &str) -> Arc<SqlMapStorage> {
        let mut maps = self.maps.lock().unwrap_or_else(PoisonError::into_inner);
        maps.entry(map_id.to_owned())
            .or_insert_with(|| Arc::new(SqlMapStorage::new(self.shared.clone(), map_id)))
            .clone()
    }

    pub fn close(&self) {
        self.shared.run(self.shared.pool.close());
    }
}

impl Storage for SqlStorage {
    fn map(&self, map_id: &str) -> Result<Arc<dyn MapStorage>> {
        match self.format {
            Format::Compat => Ok(self.sql_map(map_id)),
            Format::Optimized => Ok(self.optimized_map(map_id)),
        }
    }

    fn map_ids(&self) -> Result<Vec<String>> {
        let s = &self.shared;
        s.run(async {
            let mut ids = Vec::new();
            for page in 0.. {
                let batch = s.pool.fetch_texts(&s.sql.list_map_ids, &[Arg::Int(PAGE), Arg::Int(page * PAGE)]).await?;
                let done = (batch.len() as i64) < PAGE;
                ids.extend(batch);
                if done {
                    break;
                }
            }
            Ok(ids)
        })
    }
}

/// The URL without `user:password@`, for messages.
fn redact(url: &str) -> String {
    match (url.find("://"), url.rfind('@')) {
        (Some(s), Some(at)) if at > s => format!("{}***{}", &url[..s + 3], &url[at..]),
        _ => url.to_owned(),
    }
}

fn valid_prefix(prefix: &str) -> bool {
    prefix.len() <= 32 && prefix.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Upstream `initializeTables`: done if all six tables exist, else `CREATE TABLE IF NOT EXISTS` each.
async fn initialize_tables(pool: &Pool, dialect: Dialect, prefix: &str, read_only: bool) -> Result<()> {
    let existing: HashSet<String> = pool.fetch_texts(schema::list_tables(dialect), &[]).await?.into_iter().collect();
    let missing: Vec<String> =
        schema::TABLES.iter().map(|t| format!("{prefix}{t}")).filter(|t| !existing.contains(t)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    if read_only {
        return Err(Error::MissingTables(missing.join(", ")));
    }
    for create in schema::create_tables(dialect, prefix) {
        pool.execute(&create, &[]).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefixes() {
        assert!(valid_prefix("bluemap_") && valid_prefix(""));
        assert!(!valid_prefix("Bluemap") && !valid_prefix("a-b") && !valid_prefix(&"a".repeat(33)));
    }
}
