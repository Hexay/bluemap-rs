//! Per-dialect sqlx pools behind one small query surface. Column types differ per dialect (MySQL unsigned ids
//! and `_bin` VARCHARs, PostgreSQL INT2/BOOL, SQLite INTEGER), so decoding is dialect-aware.
//!
//! Nothing waits forever: acquiring a connection gives up after [`ACQUIRE_TIMEOUT`], a statement after
//! [`SqlConfig::statement_timeout`], closing after [`CLOSE_TIMEOUT`]. A statement that overruns leaves its
//! connection mid-protocol, so that connection is dropped instead of going back to the pool.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use sqlx::mysql::{MySqlPool, MySqlPoolOptions, MySqlRow};
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
};
// statements come from `Statements` and config (`connection-init-sql`), never from map data
use sqlx::{AssertSqlSafe, Row};

use super::{Dialect, SqlConfig};
use crate::error::{Error, Result};

/// Java's pool waits as long for a connection.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(30);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy)]
pub(crate) enum Arg<'a> {
    Int(i64),
    Str(&'a str),
    Blob(&'a [u8]),
}

pub(crate) struct Pool {
    inner: Inner,
    statement_timeout: Duration,
}

enum Inner {
    MySql(MySqlPool),
    Postgres(PgPool),
    Sqlite(SqlitePool),
}

macro_rules! bind_args {
    ($query:expr, $args:expr) => {{
        let mut q = $query;
        for arg in $args {
            q = match *arg {
                Arg::Int(v) => q.bind(v),
                Arg::Str(s) => q.bind(s),
                Arg::Blob(b) => q.bind(b),
            };
        }
        q
    }};
}

/// Runs `$body` with `$c` bound to a connection of the concrete pool and `$int`/`$text` to its decoders, under
/// the statement timeout.
macro_rules! on_conn {
    ($pool:expr, |$c:ident, $int:ident, $text:ident| $body:expr) => {
        match &$pool.inner {
            Inner::MySql(p) => {
                let ($int, $text) = (mysql_int, mysql_text);
                timed!($pool, p, |$c| $body)
            }
            Inner::Postgres(p) => {
                let ($int, $text) = (pg_int, pg_text);
                timed!($pool, p, |$c| $body)
            }
            Inner::Sqlite(p) => {
                let ($int, $text) = (sqlite_int, sqlite_text);
                timed!($pool, p, |$c| $body)
            }
        }
    };
}

macro_rules! timed {
    ($pool:expr, $p:expr, |$c:ident| $body:expr) => {{
        let mut conn = $p.acquire().await?;
        let limit = $pool.statement_timeout;
        let result = tokio::time::timeout(limit, async {
            let $c = &mut *conn;
            $body
        })
        .await;
        match result {
            Ok(result) => result,
            Err(_) => {
                drop(conn.detach());
                Err(Error::Timeout(limit))
            }
        }
    }};
}

impl Pool {
    pub async fn connect(dialect: Dialect, url: &str, config: &SqlConfig) -> Result<Self> {
        let (max_connections, read_only) = (config.max_connections.max(1), config.read_only);
        // SQLite's default pragmas are connect options; the server dialects' run as statements
        let init: Arc<[String]> = match (&config.init_sql, dialect) {
            (Some(sql), _) => sql.iter().cloned().collect(),
            (None, Dialect::Postgres) => ["SET synchronous_commit = off".to_owned()].into(),
            (None, _) => [].into(),
        };
        macro_rules! with_init {
            ($options:expr) => {{
                let init = init.clone();
                $options.max_connections(max_connections).acquire_timeout(ACQUIRE_TIMEOUT).after_connect(
                    move |conn, _| {
                        let init = init.clone();
                        Box::pin(async move {
                            for sql in init.iter() {
                                sqlx::raw_sql(AssertSqlSafe(sql.as_str())).execute(&mut *conn).await?;
                            }
                            Ok(())
                        })
                    },
                )
            }};
        }
        let inner = match dialect {
            Dialect::MySql => Inner::MySql(with_init!(MySqlPoolOptions::new()).connect(url).await?),
            Dialect::Postgres => Inner::Postgres(with_init!(PgPoolOptions::new()).connect(url).await?),
            Dialect::Sqlite => {
                let mut options = SqliteConnectOptions::from_str(url)?
                    .synchronous(SqliteSynchronous::Normal)
                    .busy_timeout(Duration::from_secs(30))
                    .foreign_keys(true)
                    .read_only(read_only)
                    .create_if_missing(!read_only);
                // switching to WAL writes the database header, which a read-only connection cannot
                if !read_only {
                    options = options.journal_mode(SqliteJournalMode::Wal);
                }
                Inner::Sqlite(with_init!(SqlitePoolOptions::new()).connect_with(options).await?)
            }
        };
        Ok(Self { inner, statement_timeout: config.statement_timeout })
    }

    pub async fn execute(&self, sql: &str, args: &[Arg<'_>]) -> Result<u64> {
        on_conn!(self, |c, _i, _t| Ok(bind_args!(sqlx::query(AssertSqlSafe(sql)), args).execute(c).await?.rows_affected()))
    }

    pub async fn fetch_blob(&self, sql: &str, args: &[Arg<'_>]) -> Result<Option<Vec<u8>>> {
        on_conn!(self, |c, _i, _t| {
            let row = bind_args!(sqlx::query(AssertSqlSafe(sql)), args).fetch_optional(c).await?;
            row.map(|r| r.try_get::<Vec<u8>, _>(0)).transpose().map_err(Error::from)
        })
    }

    /// First column of the first row as an integer (bools count as 0/1).
    pub async fn fetch_int(&self, sql: &str, args: &[Arg<'_>]) -> Result<Option<i64>> {
        on_conn!(self, |c, int, _t| {
            let row = bind_args!(sqlx::query(AssertSqlSafe(sql)), args).fetch_optional(c).await?;
            row.map(|r| int(&r, 0)).transpose()
        })
    }

    pub async fn fetch_texts(&self, sql: &str, args: &[Arg<'_>]) -> Result<Vec<String>> {
        on_conn!(self, |c, _i, text| {
            let rows = bind_args!(sqlx::query(AssertSqlSafe(sql)), args).fetch_all(c).await?;
            rows.iter().map(|r| text(r, 0)).collect()
        })
    }

    pub async fn fetch_int_pairs(&self, sql: &str, args: &[Arg<'_>]) -> Result<Vec<(i64, i64)>> {
        on_conn!(self, |c, int, _t| {
            let rows = bind_args!(sqlx::query(AssertSqlSafe(sql)), args).fetch_all(c).await?;
            rows.iter().map(|r| Ok((int(r, 0)?, int(r, 1)?))).collect()
        })
    }

    /// Runs an INSERT and returns the generated id (PostgreSQL statements carry `RETURNING id`).
    pub async fn insert_returning_id(&self, sql: &str, args: &[Arg<'_>]) -> Result<i64> {
        match &self.inner {
            Inner::MySql(p) => timed!(self, p, |c| {
                let id = bind_args!(sqlx::query(AssertSqlSafe(sql)), args).execute(c).await?.last_insert_id();
                i64::try_from(id).map_err(|_| Error::Protocol("generated id out of range"))
            }),
            Inner::Postgres(p) => timed!(self, p, |c| pg_int(&bind_args!(sqlx::query(AssertSqlSafe(sql)), args).fetch_one(c).await?, 0)),
            Inner::Sqlite(p) => {
                timed!(self, p, |c| Ok(bind_args!(sqlx::query(AssertSqlSafe(sql)), args).execute(c).await?.last_insert_rowid()))
            }
        }
    }

    /// Waits for checked-out connections to come back, but not forever (a lost one must not hang the exit).
    pub async fn close(&self) {
        let close = async {
            match &self.inner {
                Inner::MySql(p) => p.close().await,
                Inner::Postgres(p) => p.close().await,
                Inner::Sqlite(p) => p.close().await,
            }
        };
        _ = tokio::time::timeout(CLOSE_TIMEOUT, close).await;
    }
}

fn mysql_int(row: &MySqlRow, i: usize) -> Result<i64> {
    // ids are UNSIGNED, counts are signed BIGINT
    match row.try_get::<i64, _>(i) {
        Ok(v) => Ok(v),
        Err(_) => {
            let v = row.try_get::<u64, _>(i)?;
            i64::try_from(v).map_err(|_| Error::Protocol("integer out of range"))
        }
    }
}

fn mysql_text(row: &MySqlRow, i: usize) -> Result<String> {
    // utf8mb4_bin columns carry the BINARY flag, which sqlx refuses to decode as String
    String::from_utf8(row.try_get::<Vec<u8>, _>(i)?).map_err(|_| Error::Protocol("non-UTF-8 key"))
}

fn pg_int(row: &PgRow, i: usize) -> Result<i64> {
    if let Ok(v) = row.try_get::<i16, _>(i) {
        return Ok(v.into());
    }
    if let Ok(v) = row.try_get::<i32, _>(i) {
        return Ok(v.into());
    }
    if let Ok(v) = row.try_get::<bool, _>(i) {
        return Ok(v.into());
    }
    Ok(row.try_get::<i64, _>(i)?)
}

fn pg_text(row: &PgRow, i: usize) -> Result<String> {
    Ok(row.try_get::<String, _>(i)?)
}

fn sqlite_int(row: &SqliteRow, i: usize) -> Result<i64> {
    Ok(row.try_get::<i64, _>(i)?)
}

fn sqlite_text(row: &SqliteRow, i: usize) -> Result<String> {
    Ok(row.try_get::<String, _>(i)?)
}
