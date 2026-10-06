//! MySQL/MariaDB and PostgreSQL against real servers. Set `BM_TEST_MYSQL_URL` and/or `BM_TEST_POSTGRES_URL`
//! (e.g. `mysql://root:pw@localhost/bluemap`) and run `cargo test -p bm-storage --test sql_remote -- --ignored`.
//! Tables get a throwaway prefix and are dropped afterwards.

mod common;

use bm_storage::{Dialect, Error, GridKey, ItemKey, SqlConfig, SqlStorage, Storage};
use sqlx::{Connection, Executor};
use tokio::runtime::Runtime;

const TABLES_DROP_ORDER: [&str; 6] =
    ["grid_storage_data", "item_storage_data", "grid_storage", "item_storage", "compression", "map"];

fn run(env: &str) {
    let Ok(url) = std::env::var(env) else {
        eprintln!("{env} not set; skipping");
        return;
    };
    let rt = Runtime::new().unwrap();
    let prefix = format!("bmtest{}_", std::process::id());
    let mut cfg = SqlConfig::new(&url);
    cfg.table_prefix = prefix.clone();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let storage = SqlStorage::connect(&cfg, rt.handle().clone()).unwrap();
        common::conformance(&storage);

        let map = storage.map("big").unwrap();
        let blob = vec![7u8; 2 << 20];
        map.write_grid_encoded(GridKey::Hires, (0, 0), &blob).unwrap();
        assert_eq!(map.read_grid(GridKey::Hires, (0, 0)).unwrap().unwrap().data, blob);
        storage.close();

        let mut ro = cfg.clone();
        ro.read_only = true;
        let storage = SqlStorage::connect(&ro, rt.handle().clone()).unwrap();
        assert!(matches!(storage.map("big").unwrap().write_item(&ItemKey::Settings, b"{}"), Err(Error::ReadOnly)));
        storage.close();
    }));

    let (dialect, sqlx_url) = Dialect::from_url(&url).unwrap();
    rt.block_on(async {
        for table in TABLES_DROP_ORDER {
            let drop = format!("DROP TABLE IF EXISTS {prefix}{table}");
            match dialect {
                Dialect::MySql => {
                    sqlx::MySqlConnection::connect(&sqlx_url).await.unwrap().execute(drop.as_str()).await.map(drop_ok)
                }
                Dialect::Postgres => {
                    sqlx::PgConnection::connect(&sqlx_url).await.unwrap().execute(drop.as_str()).await.map(drop_ok)
                }
                Dialect::Sqlite => unreachable!(),
            }
            .unwrap();
        }
    });
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

fn drop_ok<T>(_: T) {}

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
