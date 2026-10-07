//! MySQL/MariaDB and PostgreSQL against real servers (`py -3 tools/dbs.py start` runs local ones). Set
//! `BM_TEST_MYSQL_URL` and/or `BM_TEST_POSTGRES_URL` (sqlx or JDBC URL, e.g. `mysql://root:pw@127.0.0.1/bluemap`)
//! and run `cargo test -p bm-storage --test sql_remote -- --ignored`. Tables get throwaway prefixes and are dropped
//! afterwards. The reconnect check kills every other connection to the test database. `BM_TEST_SQL_TLS=1` also
//! checks connections that require TLS (the `tools/dbs.py` servers have certificates).

mod common;

use std::sync::Arc;

use bm_storage::{Dialect, Error, Format, GridKey, ItemKey, SqlConfig, SqlStorage, Storage};
use sqlx::{Connection, Executor, Row};
use tokio::runtime::Runtime;

const TABLES_DROP_ORDER: [&str; 6] =
    ["grid_storage_data", "item_storage_data", "grid_storage", "item_storage", "compression", "map"];

struct Server {
    url: String,
    dialect: Dialect,
    sqlx_url: String,
    rt: Runtime,
}

impl Server {
    fn from_env(env: &str) -> Option<Self> {
        let Ok(url) = std::env::var(env) else {
            eprintln!("{env} not set; skipping");
            return None;
        };
        let (dialect, sqlx_url) = Dialect::from_url(&url).unwrap();
        Some(Self { url, dialect, sqlx_url, rt: Runtime::new().unwrap() })
    }

    fn config(&self, name: &str, format: Format) -> SqlConfig {
        let table_prefix = format!("bmt{}_{name}_", std::process::id());
        SqlConfig { table_prefix, format, ..SqlConfig::new(&self.url) }
    }

    fn connect(&self, config: &SqlConfig) -> SqlStorage {
        SqlStorage::connect(config, self.rt.handle().clone()).unwrap()
    }

    /// First column of the first row of `sql` (an integer) on a fresh admin connection.
    fn admin(&self, sql: &str) -> Option<i64> {
        self.rt.block_on(async {
            match self.dialect {
                Dialect::MySql => {
                    let mut c = sqlx::MySqlConnection::connect(&self.sqlx_url).await.unwrap();
                    let row = c.fetch_optional(sql).await.unwrap();
                    row.map(|r| r.try_get::<i64, _>(0).or_else(|_| r.try_get::<u64, _>(0).map(|v| v as i64)).unwrap())
                }
                Dialect::Postgres => {
                    let mut c = sqlx::PgConnection::connect(&self.sqlx_url).await.unwrap();
                    let row = c.fetch_optional(sql).await.unwrap();
                    row.and_then(|r| r.try_get::<Option<i64>, _>(0).ok().flatten())
                }
                Dialect::Sqlite => unreachable!(),
            }
        })
    }

    /// Runs `check` on storage tables `name`, then drops them (also when `check` panics).
    fn with_tables(&self, name: &str, check: impl FnOnce(&Self, &str)) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(self, name)));
        let prefix = self.config(name, Format::Compat).table_prefix;
        for table in TABLES_DROP_ORDER {
            self.admin(&format!("DROP TABLE IF EXISTS {prefix}{table}"));
        }
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    }
}

fn compat(s: &Server, name: &str) {
    let cfg = s.config(name, Format::Compat);
    let storage = s.connect(&cfg);
    common::conformance(&storage);
    storage.close();
    let ro = SqlConfig { read_only: true, ..cfg };
    let storage = s.connect(&ro);
    assert!(matches!(storage.map("m").unwrap().write_item(&ItemKey::Settings, b"{}"), Err(Error::ReadOnly)));
    storage.close();
}

fn optimized(s: &Server, name: &str) {
    let storage = s.connect(&s.config(name, Format::Optimized));
    common::conformance(&storage);
    storage.close();
}

fn conversion(s: &Server, name: &str) {
    common::sql_detection_and_conversion(&|format| s.config(name, format), s.rt.handle());
}

/// Blobs above 16 MiB (MEDIUMBLOB's limit; upstream uses LONGBLOB) round-trip where the server allows them, and a
/// blob above MySQL's `max_allowed_packet` fails cleanly before sending (#694), leaving the storage usable.
fn big_blobs(s: &Server, name: &str) {
    let storage = s.connect(&s.config(name, Format::Compat));
    let map = storage.map("big").unwrap();
    let limit = match s.dialect {
        Dialect::MySql => s.admin("SELECT @@max_allowed_packet").map(|l| l as usize),
        _ => None,
    };
    let size = match limit {
        Some(l) if l <= 20 << 20 => l - 4096,
        _ => 20 << 20,
    };
    let blob: Vec<u8> = (0..size).map(|i| (i * 31 % 251) as u8).collect();
    map.write_grid_encoded(GridKey::Hires, (0, 0), &blob).unwrap();
    assert!(map.read_grid(GridKey::Hires, (0, 0)).unwrap().unwrap().data == blob, "{size} B blob round-trips");
    if let Some(limit) = limit {
        let too_big = vec![0u8; limit + 1];
        let err = map.write_grid_encoded(GridKey::Hires, (1, 0), &too_big).unwrap_err();
        assert!(matches!(err, Error::BlobTooLarge { .. }), "{err}");
    }
    map.write_grid_encoded(GridKey::Hires, (2, 0), b"after").unwrap();
    assert_eq!(map.read_grid(GridKey::Hires, (2, 0)).unwrap().unwrap().data, b"after");
    storage.close();
}

/// Two storages ("processes") with small pools race to create the same map and keys and write 16 threads' worth of
/// tiles, some to the same cell.
fn concurrent_writers(s: &Server, name: &str) {
    let cfg = SqlConfig { max_connections: 2, ..s.config(name, Format::Compat) };
    let storages: Vec<Arc<SqlStorage>> = (0..2).map(|_| Arc::new(s.connect(&cfg))).collect();
    std::thread::scope(|scope| {
        for t in 0..16 {
            let storage = storages[t % 2].clone();
            scope.spawn(move || {
                let map = storage.map("race").unwrap();
                for i in 0..20 {
                    map.write_grid(GridKey::Hires, (t as i32, i), format!("{t}/{i}").as_bytes()).unwrap();
                    map.write_grid(GridKey::Lowres(1), (0, 0), b"shared").unwrap();
                }
            });
        }
    });
    let map = storages[0].map("race").unwrap();
    assert_eq!(map.list_grid(GridKey::Hires).unwrap().len(), 16 * 20);
    assert_eq!(map.read_grid(GridKey::Hires, (7, 19)).unwrap().unwrap().decompress().unwrap(), b"7/19");
    assert_eq!(storages[1].map_ids().unwrap(), ["race"]);
    storages.iter().for_each(|s| s.close());
}

/// Connections killed server-side (restart, idle timeout, failover) are replaced transparently.
fn reconnects(s: &Server, name: &str) {
    let storage = s.connect(&SqlConfig { max_connections: 4, ..s.config(name, Format::Compat) });
    let map = storage.map("m").unwrap();
    map.write_item(&ItemKey::Settings, b"{}").unwrap();
    let killed = match s.dialect {
        Dialect::MySql => {
            let ids = s.rt.block_on(async {
                let mut c = sqlx::MySqlConnection::connect(&s.sqlx_url).await.unwrap();
                let rows = c
                    .fetch_all("SELECT ID FROM information_schema.PROCESSLIST WHERE DB = DATABASE() AND ID <> CONNECTION_ID()")
                    .await
                    .unwrap();
                let id = |r: &sqlx::mysql::MySqlRow| r.try_get::<u64, _>(0).or_else(|_| r.try_get::<i64, _>(0).map(|v| v as u64));
                let ids: Vec<u64> = rows.iter().map(|r| id(r).unwrap()).collect();
                for id in &ids {
                    c.execute(format!("KILL {id}").as_str()).await.unwrap();
                }
                ids
            });
            ids.len() as i64
        }
        _ => s
            .admin("SELECT COUNT(pg_terminate_backend(pid)) FROM pg_stat_activity \
                    WHERE datname = current_database() AND pid <> pg_backend_pid()")
            .unwrap_or(0),
    };
    assert!(killed >= 1, "no connection to kill");
    for _ in 0..8 {
        assert_eq!(map.read_item(&ItemKey::Settings).unwrap().unwrap().data, b"{}");
        map.write_grid(GridKey::Hires, (0, 0), b"after kill").unwrap();
    }
    storage.close();
}

/// TLS (rustls) is required, not just preferred: the connection fails without it.
fn tls(s: &Server, name: &str) {
    let require = if s.dialect == Dialect::MySql { "ssl-mode=REQUIRED" } else { "sslmode=require" };
    let url = format!("{}{}{require}", s.url, if s.url.contains('?') { '&' } else { '?' });
    let storage = s.connect(&SqlConfig { url, ..s.config(name, Format::Compat) });
    let map = storage.map("m").unwrap();
    map.write_item(&ItemKey::Settings, b"{\"tls\":1}").unwrap();
    assert_eq!(map.read_item(&ItemKey::Settings).unwrap().unwrap().data, b"{\"tls\":1}");
    storage.close();
}

fn run(env: &str) {
    let Some(server) = Server::from_env(env) else { return };
    if std::env::var_os("BM_TEST_SQL_TLS").is_some() {
        server.with_tables("tls", tls);
    }
    server.with_tables("compat", compat);
    server.with_tables("opt", optimized);
    server.with_tables("conv", conversion);
    server.with_tables("big", big_blobs);
    server.with_tables("race", concurrent_writers);
    server.with_tables("kill", reconnects);
}

#[test]
#[ignore = "needs a MySQL/MariaDB server: BM_TEST_MYSQL_URL"]
fn mysql() {
    run("BM_TEST_MYSQL_URL");
}

#[test]
#[ignore = "needs a PostgreSQL server: BM_TEST_POSTGRES_URL"]
fn postgres() {
    run("BM_TEST_POSTGRES_URL");
}
