//! Find-or-create lookup ids for maps, compressions and storage keys, cached like upstream `AbstractCommandSet`.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use super::db::{Arg, Pool};
use super::statements::Statements;
use crate::error::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum KeyTable {
    Map,
    Compression,
    ItemStorage,
    GridStorage,
}

impl KeyTable {
    fn statements(self, s: &Statements) -> (&str, &str) {
        match self {
            Self::Map => (&s.find_map, &s.create_map),
            Self::Compression => (&s.find_compression, &s.create_compression),
            Self::ItemStorage => (&s.find_item_storage, &s.create_item_storage),
            Self::GridStorage => (&s.find_grid_storage, &s.create_grid_storage),
        }
    }
}

#[derive(Default)]
pub(crate) struct KeyCache {
    ids: Mutex<HashMap<(KeyTable, String), i64>>,
}

impl KeyCache {
    fn cached(&self, table: KeyTable, key: &str) -> Option<i64> {
        self.ids.lock().unwrap_or_else(PoisonError::into_inner).get(&(table, key.to_owned())).copied()
    }

    pub fn invalidate(&self, table: KeyTable, key: &str) {
        self.ids.lock().unwrap_or_else(PoisonError::into_inner).remove(&(table, key.to_owned()));
    }

    /// The id of `key`; `None` when missing and `create` is false. Only hits are cached.
    pub async fn id(
        &self,
        pool: &Pool,
        s: &Statements,
        table: KeyTable,
        key: &str,
        create: bool,
    ) -> Result<Option<i64>> {
        if let Some(id) = self.cached(table, key) {
            return Ok(Some(id));
        }
        let (find, insert) = table.statements(s);
        let mut id = pool.fetch_int(find, &[Arg::Str(key)]).await?;
        if id.is_none() && create {
            id = match pool.insert_returning_id(insert, &[Arg::Str(key)]).await {
                Ok(id) => Some(id),
                // another process inserted the same key concurrently (unique violation): use theirs
                Err(e) => Some(pool.fetch_int(find, &[Arg::Str(key)]).await?.ok_or(e)?),
            };
        }
        if let Some(id) = id {
            self.ids.lock().unwrap_or_else(PoisonError::into_inner).insert((table, key.to_owned()), id);
        }
        Ok(id)
    }
}
