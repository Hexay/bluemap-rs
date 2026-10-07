//! `BlueMapService.getOrLoadStorage`: one storage per `storages/<id>.conf`, opened on first use.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use bm_config::{BlueMapConfig, SqlStorageConfig, StorageConfig};
use bm_storage::{FileStorage, SqlConfig, SqlStorage, Storage};

use crate::convert::compression;
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
        let invalid = |e: String| Error::Invalid(format!("Failed to load and initialize the storage '{id}': {e}"));
        let storage: Arc<dyn Storage> = match storage_config {
            StorageConfig::File(c) => {
                Arc::new(FileStorage::new(c.root.clone(), compression(c.compression().map_err(invalid)?)))
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

/// JDBC URL + `connection-properties` → a sqlx URL with the credentials inlined.
fn sql_config(c: &SqlStorageConfig) -> std::result::Result<SqlConfig, String> {
    let mut url = c.connection_url.clone();
    let prop = |k: &str| c.connection_properties.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
    if let (Some(user), Some(scheme_end)) = (prop("user"), url.find("://"))
        && !url[scheme_end + 3..].contains('@')
    {
        let auth = match prop("password") {
            Some(pw) => format!("{user}:{pw}@"),
            None => format!("{user}@"),
        };
        url.insert_str(scheme_end + 3, &auth);
    }
    let mut sql = SqlConfig::new(url);
    sql.table_prefix = c.table_prefix()?.to_owned();
    sql.compression = compression(c.compression()?);
    if c.max_connections > 0 {
        sql.max_connections = c.max_connections as u32;
    }
    Ok(sql)
}
