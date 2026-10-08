mod common;

use std::path::Path;

use bm_storage::{
    Compression, Dialect, Error, FileStorage, GridKey, ItemKey, SqlConfig, SqlStorage, Storage, copy_map,
};
use sqlx::{AssertSqlSafe, Connection, Row, SqliteConnection};
use tokio::runtime::Runtime;

fn config(db: &Path) -> SqlConfig {
    SqlConfig::new(format!("sqlite:{}", db.display()))
}

/// Raw rows via a separate connection, the way sql.php or upstream would see them.
fn query_rows(rt: &Runtime, db: &Path, sql: &str) -> Vec<Vec<String>> {
    rt.block_on(async {
        let mut conn = SqliteConnection::connect(&format!("sqlite:{}", db.display())).await.unwrap();
        let rows = sqlx::query(AssertSqlSafe(sql)).fetch_all(&mut conn).await.unwrap();
        rows.iter()
            .map(|r| {
                (0..r.len())
                    .map(|i| {
                        r.try_get::<String, _>(i)
                            .or_else(|_| r.try_get::<i64, _>(i).map(|v| v.to_string()))
                            .unwrap_or_else(|_| format!("{:?}", r.try_get::<Vec<u8>, _>(i).unwrap()))
                    })
                    .collect()
            })
            .collect()
    })
}

#[test]
fn conformance() {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let storage = SqlStorage::connect(&config(&dir.path().join("bm.db")), rt.handle().clone()).unwrap();
    common::conformance(&storage);
    storage.close();
}

/// `dialect:` picks the statements, so one that contradicts the URL's driver is refused up front.
#[test]
fn dialect_must_match_url() {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("bm.db");
    let wrong = SqlConfig { dialect: Some(Dialect::Postgres), ..config(&db) };
    let err = SqlStorage::connect(&wrong, rt.handle().clone()).err().unwrap();
    assert!(matches!(err, Error::DialectMismatch { configured: "postgresql", url: "sqlite" }), "{err}");
    let right = SqlConfig { dialect: Some(Dialect::Sqlite), ..config(&db) };
    SqlStorage::connect(&right, rt.handle().clone()).unwrap().close();
}

#[test]
fn schema_keys_and_blobs_match_upstream() {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("bm.db");
    let storage = SqlStorage::connect(&config(&db), rt.handle().clone()).unwrap();
    let map = storage.map("world").unwrap();
    map.write_grid(GridKey::Hires, (-3, 4), b"prbm").unwrap();
    map.write_grid(GridKey::Lowres(2), (0, 0), b"png").unwrap();
    map.write_item(&ItemKey::Textures, b"[]").unwrap();
    storage.close();

    let tables = query_rows(
        &rt,
        &db,
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'bluemap_%' ORDER BY name",
    );
    let tables: Vec<_> = tables.into_iter().map(|r| r[0].clone()).collect();
    assert_eq!(
        tables,
        [
            "bluemap_compression",
            "bluemap_grid_storage",
            "bluemap_grid_storage_data",
            "bluemap_item_storage",
            "bluemap_item_storage_data",
            "bluemap_map"
        ]
    );
    let grid_keys = query_rows(&rt, &db, "SELECT `key` FROM bluemap_grid_storage ORDER BY id");
    assert_eq!(grid_keys, [["bluemap:hires"], ["bluemap:lowres/2"]]);
    let comp_keys = query_rows(&rt, &db, "SELECT `key` FROM bluemap_compression ORDER BY id");
    assert_eq!(comp_keys, [["bluemap:gzip"], ["bluemap:none"]]);
    let rows = query_rows(
        &rt,
        &db,
        "SELECT m.map_id, s.`key`, d.x, d.z, c.`key` FROM bluemap_grid_storage_data d \
         JOIN bluemap_map m ON d.map = m.id JOIN bluemap_grid_storage s ON d.storage = s.id \
         JOIN bluemap_compression c ON d.compression = c.id ORDER BY s.id",
    );
    assert_eq!(
        rows,
        [
            ["world", "bluemap:hires", "-3", "4", "bluemap:gzip"],
            ["world", "bluemap:lowres/2", "0", "0", "bluemap:none"]
        ]
    );

    let blob = rt.block_on(async {
        let mut conn = SqliteConnection::connect(&format!("sqlite:{}", db.display())).await.unwrap();
        sqlx::query("SELECT data FROM bluemap_grid_storage_data WHERE x = -3")
            .fetch_one(&mut conn)
            .await
            .unwrap()
            .get::<Vec<u8>, _>(0)
    });
    assert_eq!(Compression::Gzip.decompress(&blob, 1 << 20).unwrap(), b"prbm", "blob is the gzip file content");
}

#[test]
fn reads_filter_by_configured_compression() {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("bm.db");
    let gzip = SqlStorage::connect(&config(&db), rt.handle().clone()).unwrap();
    gzip.map("w").unwrap().write_grid(GridKey::Hires, (0, 0), b"t").unwrap();
    let mut zstd_cfg = config(&db);
    zstd_cfg.compression = Compression::Zstd;
    let zstd = SqlStorage::connect(&zstd_cfg, rt.handle().clone()).unwrap();
    let map = zstd.map("w").unwrap();
    assert_eq!(map.read_grid(GridKey::Hires, (0, 0)).unwrap(), None);
    assert!(map.list_grid(GridKey::Hires).unwrap().is_empty());
    // purge removes rows of every compression (#356)
    map.delete(&mut |_| true).unwrap();
    assert_eq!(gzip.map("w").unwrap().read_grid(GridKey::Hires, (0, 0)).unwrap(), None);
}

#[test]
fn read_only_never_creates_anything() {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("bm.db");
    let mut ro = config(&db);
    ro.read_only = true;
    assert!(SqlStorage::connect(&ro, rt.handle().clone()).is_err(), "missing database file");

    let rw = SqlStorage::connect(&config(&db), rt.handle().clone()).unwrap();
    rw.map("w").unwrap().write_item(&ItemKey::Settings, b"{}").unwrap();
    rw.close();

    let storage = SqlStorage::connect(&ro, rt.handle().clone()).unwrap();
    let map = storage.map("w").unwrap();
    assert_eq!(map.read_item(&ItemKey::Settings).unwrap().unwrap().data, b"{}");
    assert_eq!(map.read_grid(GridKey::Lowres(7), (0, 0)).unwrap(), None);
    assert!(matches!(map.write_item(&ItemKey::Settings, b"x"), Err(Error::ReadOnly)));
    assert_eq!(storage.map("nope").unwrap().read_item(&ItemKey::Settings).unwrap(), None);
    let grid_keys = query_rows(&rt, &db, "SELECT COUNT(*) FROM bluemap_grid_storage");
    assert_eq!(grid_keys, [["0"]], "lookups must not insert keys");
}

#[test]
fn read_only_reports_missing_tables() {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("empty.db");
    rt.block_on(async {
        SqliteConnection::connect(&format!("sqlite:{}?mode=rwc", db.display())).await.unwrap().close().await
    })
    .unwrap();
    let mut ro = config(&db);
    ro.read_only = true;
    assert!(matches!(SqlStorage::connect(&ro, rt.handle().clone()), Err(Error::MissingTables(_))));
}

#[test]
fn rejects_bad_prefix() {
    let rt = Runtime::new().unwrap();
    let mut cfg = config(Path::new("unused.db"));
    cfg.table_prefix = "Bad-Prefix".into();
    assert!(matches!(SqlStorage::connect(&cfg, rt.handle().clone()), Err(Error::InvalidTablePrefix(_))));
}

/// file → SQLite → file reproduces the original tree byte for byte.
#[test]
fn file_sql_file_round_trip_is_identical() {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let src = FileStorage::new(dir.path().join("a"), Compression::Gzip);
    let map = src.map("m").unwrap();
    for grid in common::GRIDS {
        map.write_grid(grid, (-5, 17), format!("{grid:?}").as_bytes()).unwrap();
    }
    map.write_item(&ItemKey::Textures, b"[1]").unwrap();
    map.write_item(&ItemKey::Settings, b"{}").unwrap();
    map.write_item(&ItemKey::asset("a/b.png"), b"png").unwrap();

    let sql = SqlStorage::connect(&config(&dir.path().join("bm.db")), rt.handle().clone()).unwrap();
    let stats = copy_map(map.as_ref(), sql.map("m").unwrap().as_ref()).unwrap();
    assert_eq!((stats.cells, stats.items, stats.transcoded), (6, 3, 0));
    let dst = FileStorage::new(dir.path().join("b"), Compression::Gzip);
    copy_map(sql.map("m").unwrap().as_ref(), dst.map("m").unwrap().as_ref()).unwrap();
    common::assert_same_tree(&dir.path().join("a"), &dir.path().join("b"));
}
