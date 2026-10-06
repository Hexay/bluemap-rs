//! Data statements, semantically equal to upstream's command sets. Templates use MySQL/SQLite quoting and `?`;
//! PostgreSQL drops the backticks and numbers its placeholders.

use super::Dialect;

/// Resolved statements for one dialect and table prefix. Parameter order is given per field.
#[derive(Debug, Clone)]
pub(crate) struct Statements {
    /// map, storage, compression, data
    pub item_write: String,
    /// map, storage, compression
    pub item_read: String,
    /// map, storage
    pub item_delete: String,
    /// map, storage, compression
    pub item_has: String,
    /// map → asset keys
    pub item_list_assets: String,
    /// map, storage, x, z, compression, data
    pub grid_write: String,
    /// map, storage, x, z, compression
    pub grid_read: String,
    /// map, storage, x, z
    pub grid_delete: String,
    /// map, storage, x, z, compression
    pub grid_has: String,
    /// map, storage, compression, limit, offset
    pub grid_list: String,
    /// map → grid storage keys with data
    pub grid_list_storages: String,
    /// map
    pub grid_count_map: String,
    /// map, limit
    pub grid_purge_map: String,
    /// map id (numeric)
    pub purge_map: String,
    /// map_id
    pub has_map: String,
    /// limit, offset
    pub list_map_ids: String,
    /// lookup tables: find by key / insert key (returns generated id)
    pub find_map: String,
    pub create_map: String,
    pub find_compression: String,
    pub create_compression: String,
    pub find_item_storage: String,
    pub create_item_storage: String,
    pub find_grid_storage: String,
    pub create_grid_storage: String,
}

impl Statements {
    pub fn new(dialect: Dialect, prefix: &str) -> Self {
        let s = |template: &str| resolve(dialect, prefix, template);
        let (item_write, grid_write, grid_purge_map) = match dialect {
            Dialect::MySql | Dialect::Sqlite => (
                s(
                    "REPLACE INTO `${prefix}item_storage_data` (`map`, `storage`, `compression`, `data`) VALUES (?, ?, ?, ?)",
                ),
                s("REPLACE INTO `${prefix}grid_storage_data` (`map`, `storage`, `x`, `z`, `compression`, `data`) \
                   VALUES (?, ?, ?, ?, ?, ?)"),
                if dialect == Dialect::MySql {
                    s("DELETE FROM `${prefix}grid_storage_data` WHERE `map` = ? LIMIT ?")
                } else {
                    s("DELETE FROM `${prefix}grid_storage_data` WHERE ROWID IN \
                       (SELECT t.ROWID FROM `${prefix}grid_storage_data` t WHERE t.`map` = ? LIMIT ?)")
                },
            ),
            Dialect::Postgres => (
                s("INSERT INTO ${prefix}item_storage_data (map, storage, compression, data) VALUES (?, ?, ?, ?) \
                   ON CONFLICT (map, storage) DO UPDATE SET compression = excluded.compression, data = excluded.data"),
                s("INSERT INTO ${prefix}grid_storage_data (map, storage, x, z, compression, data) \
                   VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (map, storage, x, z) \
                   DO UPDATE SET compression = excluded.compression, data = excluded.data"),
                s("DELETE FROM ${prefix}grid_storage_data WHERE CTID IN \
                   (SELECT CTID FROM ${prefix}grid_storage_data t WHERE t.map = ? LIMIT ?)"),
            ),
        };
        let create = |table: &str, col: &str| {
            let insert = s(&format!("INSERT INTO `${{prefix}}{table}` (`{col}`) VALUES (?)"));
            // JDBC getGeneratedKeys does this for PostgreSQL; the other drivers report the id directly
            if dialect == Dialect::Postgres { insert + " RETURNING id" } else { insert }
        };
        let find = |table: &str, col: &str| s(&format!("SELECT `id` FROM `${{prefix}}{table}` WHERE `{col}` = ?"));
        Self {
            item_write,
            item_read: s("SELECT `data` FROM `${prefix}item_storage_data` \
                          WHERE `map` = ? AND `storage` = ? AND `compression` = ?"),
            item_delete: s("DELETE FROM `${prefix}item_storage_data` WHERE `map` = ? AND `storage` = ?"),
            item_has: s("SELECT COUNT(*) > 0 FROM `${prefix}item_storage_data` \
                         WHERE `map` = ? AND `storage` = ? AND `compression` = ?"),
            item_list_assets: s("SELECT s.`key` FROM `${prefix}item_storage_data` d \
                                 JOIN `${prefix}item_storage` s ON d.`storage` = s.`id` \
                                 WHERE d.`map` = ? AND s.`key` LIKE 'bluemap:asset/%' ORDER BY s.`key`"),
            grid_write,
            grid_read: s("SELECT `data` FROM `${prefix}grid_storage_data` \
                          WHERE `map` = ? AND `storage` = ? AND `x` = ? AND `z` = ? AND `compression` = ?"),
            grid_delete: s("DELETE FROM `${prefix}grid_storage_data` \
                            WHERE `map` = ? AND `storage` = ? AND `x` = ? AND `z` = ?"),
            grid_has: s("SELECT COUNT(*) > 0 FROM `${prefix}grid_storage_data` \
                         WHERE `map` = ? AND `storage` = ? AND `x` = ? AND `z` = ? AND `compression` = ?"),
            // ORDER BY follows the primary key, so paging is stable and index-only
            grid_list: s("SELECT `x`, `z` FROM `${prefix}grid_storage_data` \
                          WHERE `map` = ? AND `storage` = ? AND `compression` = ? ORDER BY `x`, `z` LIMIT ? OFFSET ?"),
            grid_list_storages: s("SELECT s.`key` FROM `${prefix}grid_storage` s WHERE EXISTS \
                                   (SELECT 1 FROM `${prefix}grid_storage_data` d \
                                   WHERE d.`map` = ? AND d.`storage` = s.`id`) ORDER BY s.`key`"),
            grid_count_map: s("SELECT COUNT(*) FROM `${prefix}grid_storage_data` WHERE `map` = ?"),
            grid_purge_map,
            purge_map: s("DELETE FROM `${prefix}map` WHERE `id` = ?"),
            has_map: s("SELECT COUNT(*) > 0 FROM `${prefix}map` m WHERE m.`map_id` = ?"),
            list_map_ids: s("SELECT `map_id` FROM `${prefix}map` m ORDER BY `id` LIMIT ? OFFSET ?"),
            find_map: find("map", "map_id"),
            create_map: create("map", "map_id"),
            find_compression: find("compression", "key"),
            create_compression: create("compression", "key"),
            find_item_storage: find("item_storage", "key"),
            create_item_storage: create("item_storage", "key"),
            find_grid_storage: find("grid_storage", "key"),
            create_grid_storage: create("grid_storage", "key"),
        }
    }
}

fn resolve(dialect: Dialect, prefix: &str, template: &str) -> String {
    let sql = template.replace("${prefix}", prefix);
    if dialect != Dialect::Postgres {
        return sql;
    }
    let mut out = String::with_capacity(sql.len() + 8);
    let mut n = 0;
    for c in sql.chars() {
        match c {
            '`' => {}
            '?' => {
                n += 1;
                out.push_str(&format!("${n}"));
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_statements() {
        let s = Statements::new(Dialect::MySql, "bluemap_");
        assert_eq!(
            s.grid_write,
            "REPLACE INTO `bluemap_grid_storage_data` (`map`, `storage`, `x`, `z`, `compression`, `data`) VALUES (?, ?, ?, ?, ?, ?)"
        );
        assert_eq!(s.grid_purge_map, "DELETE FROM `bluemap_grid_storage_data` WHERE `map` = ? LIMIT ?");
        assert_eq!(s.create_map, "INSERT INTO `bluemap_map` (`map_id`) VALUES (?)");
        assert_eq!(s.find_compression, "SELECT `id` FROM `bluemap_compression` WHERE `key` = ?");
    }

    #[test]
    fn postgres_statements() {
        let s = Statements::new(Dialect::Postgres, "bluemap_");
        assert_eq!(
            s.grid_read,
            "SELECT data FROM bluemap_grid_storage_data WHERE map = $1 AND storage = $2 AND x = $3 AND z = $4 AND compression = $5"
        );
        assert_eq!(
            s.item_write,
            "INSERT INTO bluemap_item_storage_data (map, storage, compression, data) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (map, storage) DO UPDATE SET compression = excluded.compression, data = excluded.data"
        );
        assert_eq!(s.create_grid_storage, "INSERT INTO bluemap_grid_storage (key) VALUES ($1) RETURNING id");
        assert!(
            s.grid_purge_map
                .contains("CTID IN (SELECT CTID FROM bluemap_grid_storage_data t WHERE t.map = $1 LIMIT $2)")
        );
        assert!(s.item_list_assets.contains("LIKE 'bluemap:asset/%'"));
    }

    #[test]
    fn sqlite_statements() {
        let s = Statements::new(Dialect::Sqlite, "");
        assert!(s.grid_purge_map.starts_with("DELETE FROM `grid_storage_data` WHERE ROWID IN (SELECT t.ROWID"));
        assert_eq!(s.has_map, "SELECT COUNT(*) > 0 FROM `map` m WHERE m.`map_id` = ?");
    }
}
