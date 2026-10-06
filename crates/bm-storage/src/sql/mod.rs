//! SQL storage (MySQL/MariaDB, PostgreSQL, SQLite) with upstream's schema, keys and blob contents, so existing
//! databases and `sql.php` keep working. Blocking API over sqlx: calls `Handle::block_on`, so call it from plain
//! threads (render workers, `spawn_blocking`), never from inside an async task; the runtime must be multi-thread.

mod db;
mod keys;
mod map;
mod schema;
mod statements;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};

use bm_compress::Compression;
use tokio::runtime::Handle;

use self::db::{Arg, Pool};
use self::keys::{KeyCache, KeyTable};
pub use self::map::SqlMapStorage;
use self::statements::Statements;
use crate::api::{MapStorage, Storage};
use crate::error::{Error, Result};

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

impl Dialect {
    /// Dialect and sqlx URL for a sqlx-style or BlueMap JDBC-style (`jdbc:mysql://…`) connection URL.
    pub fn from_url(url: &str) -> Result<(Self, String)> {
        let bare = url.strip_prefix("jdbc:").unwrap_or(url);
        let (scheme, rest) = bare.split_once(':').ok_or_else(|| Error::UnsupportedUrl(url.to_owned()))?;
        let dialect = match scheme {
            "mysql" | "mariadb" => Self::MySql,
            "postgres" | "postgresql" => Self::Postgres,
            "sqlite" => Self::Sqlite,
            _ => return Err(Error::UnsupportedUrl(url.to_owned())),
        };
        let scheme = if dialect == Self::MySql { "mysql" } else { scheme };
        Ok((dialect, format!("{scheme}:{rest}")))
    }
}

#[derive(Debug, Clone)]
pub struct SqlConfig {
    /// sqlx or JDBC URL including credentials.
    pub url: String,
    /// Must match `[a-z0-9_]{0,32}`; `sql.php` hardcodes the default.
    pub table_prefix: String,
    pub compression: Compression,
    /// Never creates tables or keys and refuses writes (webserver-only setups, #749).
    pub read_only: bool,
    pub max_connections: u32,
}

impl SqlConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            table_prefix: "bluemap_".into(),
            compression: Compression::Gzip,
            read_only: false,
            max_connections: 8,
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
    maps: Mutex<HashMap<String, Arc<SqlMapStorage>>>,
}

impl SqlStorage {
    /// Connects and makes sure the six tables exist (creating them unless read-only).
    pub fn connect(config: &SqlConfig, runtime: Handle) -> Result<Self> {
        if !valid_prefix(&config.table_prefix) {
            return Err(Error::InvalidTablePrefix(config.table_prefix.clone()));
        }
        let (dialect, url) = Dialect::from_url(&config.url)?;
        let prefix = config.table_prefix.as_str();
        let (pool, max_packet) = runtime.block_on(async {
            let pool = Pool::connect(dialect, &url, config.max_connections.max(1), config.read_only).await?;
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
        Ok(Self { shared: Arc::new(shared), maps: Mutex::default() })
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
        Ok(self.sql_map(map_id))
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
    fn urls_and_prefixes() {
        assert_eq!(Dialect::from_url("jdbc:mysql://h:3306/db").unwrap(), (Dialect::MySql, "mysql://h:3306/db".into()));
        assert_eq!(Dialect::from_url("jdbc:mariadb://h/db").unwrap(), (Dialect::MySql, "mysql://h/db".into()));
        assert_eq!(Dialect::from_url("jdbc:postgresql://h/db").unwrap().0, Dialect::Postgres);
        assert_eq!(Dialect::from_url("sqlite:bluemap.db").unwrap(), (Dialect::Sqlite, "sqlite:bluemap.db".into()));
        assert!(Dialect::from_url("jdbc:oracle:thin:@h").is_err());
        assert!(valid_prefix("bluemap_") && valid_prefix(""));
        assert!(!valid_prefix("Bluemap") && !valid_prefix("a-b") && !valid_prefix(&"a".repeat(33)));
    }
}
