//! One map in a SQL storage (upstream `SQLMapStorage` + `SQLGridStorage` + `SQLItemStorage`).

use std::sync::Arc;

use bm_compress::Compression;
use bm_format::grid::Tile;

use super::db::Arg;
use super::keys::KeyTable;
use super::{PAGE, Shared};
use crate::api::{MapStorage, Stored};
use crate::error::{Error, Result};
use crate::key::{ASSET_KEY_PREFIX, GridKey, ItemKey};
use crate::locks::KeyLocks;

pub struct SqlMapStorage {
    shared: Arc<Shared>,
    id: String,
    locks: KeyLocks,
}

/// Ids of (map, storage key, compression); `None` if any is unknown, so nothing can be stored under them.
type Ids = Option<(i64, i64, i64)>;

impl SqlMapStorage {
    pub(super) fn new(shared: Arc<Shared>, id: &str) -> Self {
        Self { shared, id: id.to_owned(), locks: KeyLocks::default() }
    }

    async fn ids(&self, table: KeyTable, key: &str, compression: &str, create: bool) -> Result<Ids> {
        let s = &self.shared;
        let Some(map) = s.key_id(KeyTable::Map, &self.id, create).await? else { return Ok(None) };
        let Some(storage) = s.key_id(table, key, create).await? else { return Ok(None) };
        let Some(comp) = s.key_id(KeyTable::Compression, compression, create).await? else { return Ok(None) };
        Ok(Some((map, storage, comp)))
    }

    async fn item_ids(&self, item: &ItemKey, create: bool) -> Result<Ids> {
        self.ids(KeyTable::ItemStorage, &item.sql_key(), self.item_compression(item).key(), create).await
    }

    fn map_id_num(&self) -> Result<Option<i64>> {
        self.shared.run(self.shared.key_id(KeyTable::Map, &self.id, false))
    }

    /// A grid cell by raw storage and compression keys (`GridKey` cells use `sql_key()` and the compression key).
    pub(crate) fn read_cell(&self, key: &str, compression: &str, tile: Tile) -> Result<Option<Vec<u8>>> {
        let s = &self.shared;
        s.run(async {
            let Some((m, g, c)) = self.ids(KeyTable::GridStorage, key, compression, false).await? else {
                return Ok(None);
            };
            let [x, z] = cell(tile);
            s.pool.fetch_blob(&s.sql.grid_read, &[Arg::Int(m), Arg::Int(g), x, z, Arg::Int(c)]).await
        })
    }

    pub(crate) fn write_cell(&self, key: &str, compression: &str, tile: Tile, data: &[u8]) -> Result<()> {
        let s = &self.shared;
        s.check_writable(data.len())?;
        s.run(async {
            let ids = self.ids(KeyTable::GridStorage, key, compression, true).await?;
            let (m, g, c) = ids.ok_or(Error::Protocol("key not created"))?;
            let [x, z] = cell(tile);
            let args = [Arg::Int(m), Arg::Int(g), x, z, Arg::Int(c), Arg::Blob(data)];
            s.pool.execute(&s.sql.grid_write, &args).await.map(drop)
        })
    }

    /// Deletes the cell whatever its compression.
    pub(crate) fn delete_cell(&self, key: &str, tile: Tile) -> Result<()> {
        let s = &self.shared;
        s.check_writable(0)?;
        s.run(async {
            let Some(m) = s.key_id(KeyTable::Map, &self.id, false).await? else { return Ok(()) };
            let Some(g) = s.key_id(KeyTable::GridStorage, key, false).await? else { return Ok(()) };
            let [x, z] = cell(tile);
            s.pool.execute(&s.sql.grid_delete, &[Arg::Int(m), Arg::Int(g), x, z]).await.map(drop)
        })
    }

    pub(crate) fn has_cell(&self, key: &str, compression: &str, tile: Tile) -> Result<bool> {
        let s = &self.shared;
        s.run(async {
            let Some((m, g, c)) = self.ids(KeyTable::GridStorage, key, compression, false).await? else {
                return Ok(false);
            };
            let [x, z] = cell(tile);
            let n = s.pool.fetch_int(&s.sql.grid_has, &[Arg::Int(m), Arg::Int(g), x, z, Arg::Int(c)]).await?;
            Ok(n.unwrap_or(0) != 0)
        })
    }

    pub(crate) fn list_cells(&self, key: &str, compression: &str) -> Result<Vec<Tile>> {
        let s = &self.shared;
        s.run(async {
            let Some((m, g, c)) = self.ids(KeyTable::GridStorage, key, compression, false).await? else {
                return Ok(Vec::new());
            };
            let mut tiles = Vec::new();
            for page in 0.. {
                let args = [Arg::Int(m), Arg::Int(g), Arg::Int(c), Arg::Int(PAGE), Arg::Int(page * PAGE)];
                let batch = s.pool.fetch_int_pairs(&s.sql.grid_list, &args).await?;
                let done = (batch.len() as i64) < PAGE;
                for (x, z) in batch {
                    let coord = |v: i64| i32::try_from(v).map_err(|_| Error::Protocol("cell coordinate out of range"));
                    tiles.push((coord(x)?, coord(z)?));
                }
                if done {
                    break;
                }
            }
            Ok(tiles)
        })
    }

    /// Deletes every cell of storage `key` of this map, in any compression.
    pub(crate) fn delete_cells(&self, key: &str) -> Result<()> {
        let s = &self.shared;
        s.check_writable(0)?;
        s.run(async {
            let Some(m) = s.key_id(KeyTable::Map, &self.id, false).await? else { return Ok(()) };
            let Some(g) = s.key_id(KeyTable::GridStorage, key, false).await? else { return Ok(()) };
            s.pool.execute(&s.sql.grid_purge_storage, &[Arg::Int(m), Arg::Int(g)]).await.map(drop)
        })
    }
}

fn cell(tile: Tile) -> [Arg<'static>; 2] {
    [Arg::Int(tile.0.into()), Arg::Int(tile.1.into())]
}

impl MapStorage for SqlMapStorage {
    fn map_id(&self) -> &str {
        &self.id
    }

    fn compression(&self) -> Compression {
        self.shared.compression
    }

    fn read_grid(&self, grid: GridKey, tile: Tile) -> Result<Option<Stored>> {
        let compression = self.grid_compression(grid);
        let data = self.read_cell(&grid.sql_key(), compression.key(), tile)?;
        Ok(data.map(|data| Stored { data, compression }))
    }

    fn write_grid_encoded(&self, grid: GridKey, tile: Tile, encoded: &[u8]) -> Result<()> {
        self.write_cell(&grid.sql_key(), self.grid_compression(grid).key(), tile, encoded)
    }

    fn delete_grid(&self, grid: GridKey, tile: Tile) -> Result<()> {
        self.delete_cell(&grid.sql_key(), tile)
    }

    fn grid_exists(&self, grid: GridKey, tile: Tile) -> Result<bool> {
        self.has_cell(&grid.sql_key(), self.grid_compression(grid).key(), tile)
    }

    fn list_grid(&self, grid: GridKey) -> Result<Vec<Tile>> {
        self.list_cells(&grid.sql_key(), self.grid_compression(grid).key())
    }

    fn grids(&self) -> Result<Vec<GridKey>> {
        let s = &self.shared;
        let Some(m) = self.map_id_num()? else { return Ok(Vec::new()) };
        let keys = s.run(s.pool.fetch_texts(&s.sql.grid_list_storages, &[Arg::Int(m)]))?;
        let mut grids: Vec<GridKey> = keys.iter().filter_map(|k| GridKey::from_sql_key(k)).collect();
        grids.sort();
        Ok(grids)
    }

    fn read_item(&self, item: &ItemKey) -> Result<Option<Stored>> {
        let s = &self.shared;
        let compression = self.item_compression(item);
        s.run(async {
            let Some((m, i, c)) = self.item_ids(item, false).await? else { return Ok(None) };
            let data = s.pool.fetch_blob(&s.sql.item_read, &[Arg::Int(m), Arg::Int(i), Arg::Int(c)]).await?;
            Ok(data.map(|data| Stored { data, compression }))
        })
    }

    fn write_item_encoded(&self, item: &ItemKey, encoded: &[u8]) -> Result<()> {
        let s = &self.shared;
        s.check_writable(encoded.len())?;
        s.run(async {
            let (m, i, c) = self.item_ids(item, true).await?.ok_or(Error::Protocol("key not created"))?;
            let args = [Arg::Int(m), Arg::Int(i), Arg::Int(c), Arg::Blob(encoded)];
            s.pool.execute(&s.sql.item_write, &args).await.map(drop)
        })
    }

    fn delete_item(&self, item: &ItemKey) -> Result<()> {
        let s = &self.shared;
        s.check_writable(0)?;
        s.run(async {
            let Some((m, i, _)) = self.item_ids(item, false).await? else { return Ok(()) };
            s.pool.execute(&s.sql.item_delete, &[Arg::Int(m), Arg::Int(i)]).await.map(drop)
        })
    }

    fn item_exists(&self, item: &ItemKey) -> Result<bool> {
        let s = &self.shared;
        s.run(async {
            let Some((m, i, c)) = self.item_ids(item, false).await? else { return Ok(false) };
            let n = s.pool.fetch_int(&s.sql.item_has, &[Arg::Int(m), Arg::Int(i), Arg::Int(c)]).await?;
            Ok(n.unwrap_or(0) != 0)
        })
    }

    fn list_assets(&self) -> Result<Vec<String>> {
        let s = &self.shared;
        let Some(m) = self.map_id_num()? else { return Ok(Vec::new()) };
        let keys = s.run(s.pool.fetch_texts(&s.sql.item_list_assets, &[Arg::Int(m)]))?;
        Ok(keys.into_iter().filter_map(|k| k.strip_prefix(ASSET_KEY_PREFIX).map(str::to_owned)).collect())
    }

    /// Upstream: purge grid rows 1000 at a time for progress, then delete the map row (cascading to items).
    /// Rows of every storage and compression go, not just the configured ones (#356).
    fn delete(&self, on_progress: &mut dyn FnMut(f64) -> bool) -> Result<()> {
        let s = &self.shared;
        s.check_writable(0)?;
        let Some(m) = self.map_id_num()? else { return Ok(()) };
        let total = s.run(s.pool.fetch_int(&s.sql.grid_count_map, &[Arg::Int(m)]))?.unwrap_or(0);
        let mut deleted = 0;
        while deleted < total {
            let n = s.run(s.pool.execute(&s.sql.grid_purge_map, &[Arg::Int(m), Arg::Int(PAGE)]))?;
            deleted += n as i64;
            if !on_progress(deleted as f64 / total as f64) {
                return Ok(());
            }
            if n == 0 {
                break;
            }
        }
        s.run(s.pool.execute(&s.sql.purge_map, &[Arg::Int(m)]))?;
        s.keys.invalidate(KeyTable::Map, &self.id);
        Ok(())
    }

    fn exists(&self) -> Result<bool> {
        let s = &self.shared;
        let n = s.run(s.pool.fetch_int(&s.sql.has_map, &[Arg::Str(&self.id)]))?;
        Ok(n.unwrap_or(0) != 0)
    }

    fn key_locks(&self) -> &KeyLocks {
        &self.locks
    }
}
