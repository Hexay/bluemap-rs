//! `BlueMapService.getOrLoadStorage`: one storage per `storages/<id>.conf`, opened on first use.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use bm_config::{BlueMapConfig, SqlStorageConfig, StorageConfig};
use bm_storage::{
    ConvertStats, FileStorage, Format, Progress, SqlConfig, SqlStorage, Storage, convert_file_storage,
    convert_sql_storage,
};

use crate::convert::{compression, format};
use crate::error::{Error, Result};

#[derive(Default)]
pub struct Storages {
    open: Mutex<HashMap<String, Arc<dyn Storage>>>,
    /// Only started when a SQL storage is used; sqlx needs a multi-thread runtime.
    runtime: OnceLock<tokio::runtime::Runtime>,
}

impl Storages {
    pub fn get(&self, config: &BlueMapConfig, id: &str) -> Result<Arc<dyn Storage>> {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(s) = open.get(id) {
            return Ok(s.clone());
        }
        let storage_config = config.storages.get(id).ok_or_else(|| {
            Error::Invalid(format!(
                "There is no storage-configuration for '{id}'!\nYou will either need to define that storage, or change \
                 the map-config to use a storage-config that exists."
            ))
        })?;
        let invalid = |e: String| {
            let e = e.replace("<storage-id>", id);
            Error::Invalid(format!("Failed to load and initialize the storage '{id}': {e}"))
        };
        let storage: Arc<dyn Storage> = match storage_config {
            StorageConfig::File(c) => {
                let compression = compression(c.compression().map_err(invalid)?);
                let opened = FileStorage::open(c.root.clone(), compression, format(c.format), false);
                Arc::new(opened.map_err(|e| invalid(e.to_string()))?)
            }
            StorageConfig::Sql(c) => {
                let sql = sql_config(c).map_err(invalid)?;
                let runtime = self.runtime()?;
                Arc::new(SqlStorage::connect(&sql, runtime.handle().clone()).map_err(|e| invalid(e.to_string()))?)
            }
        };
        open.insert(id.to_owned(), storage.clone());
        Ok(storage)
    }

    /// Converts storage `id` in place to `to` (see `bm_storage::convert_file_storage`). It must not be open.
    pub fn convert(&self, config: &BlueMapConfig, id: &str, to: Format, progress: Progress) -> Result<ConvertStats> {
        let grids = |map: &str| config.maps.get(map).map(crate::map::hires_grid);
        let storage_config = config
            .storages
            .get(id)
            .ok_or_else(|| Error::Invalid(format!("There is no storage-configuration for '{id}'!")))?;
        let invalid = |e: String| Error::Invalid(format!("Failed to convert the storage '{id}': {e}"));
        match storage_config {
            StorageConfig::File(c) => {
                let compression = compression(c.compression().map_err(invalid)?);
                convert_file_storage(&c.root, compression, to, &grids, progress).map_err(|e| invalid(e.to_string()))
            }
            StorageConfig::Sql(c) => {
                let sql = sql_config(c).map_err(invalid)?;
                let handle = self.runtime()?.handle().clone();
                convert_sql_storage(&sql, handle, to, &grids, progress).map_err(|e| invalid(e.to_string()))
            }
        }
    }

    fn runtime(&self) -> Result<&tokio::runtime::Runtime> {
        if let Some(rt) = self.runtime.get() {
            return Ok(rt);
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| Error::Invalid(format!("failed to start the SQL runtime: {e}")))?;
        Ok(self.runtime.get_or_init(|| rt))
    }
}

fn sql_config(c: &SqlStorageConfig) -> std::result::Result<SqlConfig, String> {
    let mut sql = SqlConfig::new(&c.connection_url);
    sql.properties = c.connection_properties.clone();
    sql.init_sql = c.connection_init_sql.clone();
    sql.table_prefix = c.table_prefix()?.to_owned();
    sql.compression = compression(c.compression()?);
    sql.format = format(c.format);
    if c.dialect.is_some() {
        sql.dialect = Some(match c.dialect()? {
            bm_config::Dialect::Mysql | bm_config::Dialect::Mariadb => bm_storage::Dialect::MySql,
            bm_config::Dialect::Postgresql => bm_storage::Dialect::Postgres,
            bm_config::Dialect::Sqlite => bm_storage::Dialect::Sqlite,
        });
    }
    // Java: negative = unbounded (bm_storage::UNBOUNDED_CONNECTIONS, SqlConfig's default)
    if c.max_connections > 0 {
        sql.max_connections = c.max_connections as u32;
    }
    Ok(sql)
}
