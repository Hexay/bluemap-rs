//! Table definitions, verbatim from upstream `MySQLCommandSet`, `PostgreSQLCommandSet` and `SqliteCommandSet`
//! (only `${prefix}` substituted), so databases created by either implementation are identical.

use super::Dialect;

pub(crate) const TABLES: [&str; 6] =
    ["map", "compression", "item_storage", "item_storage_data", "grid_storage", "grid_storage_data"];

pub(crate) fn list_tables(dialect: Dialect) -> &'static str {
    match dialect {
        Dialect::MySql => {
            "SELECT TABLE_NAME FROM information_schema.tables WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE'"
        }
        Dialect::Postgres => "SELECT tablename FROM pg_catalog.pg_tables WHERE schemaname = current_schema()",
        Dialect::Sqlite => "SELECT `name` FROM `sqlite_master` WHERE `type` = 'table'",
    }
}

/// `CREATE TABLE IF NOT EXISTS` for all six tables, in dependency order.
pub(crate) fn create_tables(dialect: Dialect, prefix: &str) -> Vec<String> {
    let templates = match dialect {
        Dialect::MySql => mysql(),
        Dialect::Postgres => postgres(),
        Dialect::Sqlite => sqlite(),
    };
    templates.iter().map(|t| t.replace("${prefix}", prefix)).collect()
}

fn key_table(dialect: Dialect, name: &str, id_type: &str) -> String {
    match dialect {
        Dialect::MySql => format!(
            "CREATE TABLE IF NOT EXISTS `${{prefix}}{name}` (\n `id` {id_type} UNSIGNED NOT NULL AUTO_INCREMENT,\n \
             `{col}` VARCHAR(190) NOT NULL,\n PRIMARY KEY (`id`),\n UNIQUE INDEX `{col}` (`{col}`)\n) COLLATE 'utf8mb4_bin'",
            col = if name == "map" { "map_id" } else { "key" },
        ),
        Dialect::Postgres => format!(
            "CREATE TABLE IF NOT EXISTS ${{prefix}}{name} (\n id {id_type} PRIMARY KEY,\n {col} VARCHAR(190) UNIQUE NOT NULL\n)",
            col = if name == "map" { "map_id" } else { "key" },
        ),
        Dialect::Sqlite => format!(
            "CREATE TABLE IF NOT EXISTS `${{prefix}}{name}` (\n `id` INTEGER PRIMARY KEY AUTOINCREMENT,\n \
             `{col}` TEXT UNIQUE NOT NULL\n) STRICT",
            col = if name == "map" { "map_id" } else { "key" },
        ),
    }
}

fn mysql() -> Vec<String> {
    let d = Dialect::MySql;
    vec![
        key_table(d, "map", "SMALLINT"),
        key_table(d, "compression", "SMALLINT"),
        key_table(d, "item_storage", "INT"),
        "CREATE TABLE IF NOT EXISTS `${prefix}item_storage_data` (\n `map` SMALLINT UNSIGNED NOT NULL,\n \
         `storage` INT UNSIGNED NOT NULL,\n `compression` SMALLINT UNSIGNED NOT NULL,\n `data` LONGBLOB NOT NULL,\n \
         PRIMARY KEY (`map`, `storage`),\n CONSTRAINT `fk_${prefix}item_map`\n  FOREIGN KEY (`map`)\n  \
         REFERENCES `${prefix}map` (`id`)\n  ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n \
         CONSTRAINT `fk_${prefix}item`\n  FOREIGN KEY (`storage`)\n  REFERENCES `${prefix}item_storage` (`id`)\n  \
         ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n CONSTRAINT `fk_${prefix}item_compression`\n  \
         FOREIGN KEY (`compression`)\n  REFERENCES `${prefix}compression` (`id`)\n  ON UPDATE RESTRICT\n  \
         ON DELETE CASCADE\n) COLLATE 'utf8mb4_bin'"
            .into(),
        key_table(d, "grid_storage", "SMALLINT"),
        "CREATE TABLE IF NOT EXISTS `${prefix}grid_storage_data` (\n `map` SMALLINT UNSIGNED NOT NULL,\n \
         `storage` SMALLINT UNSIGNED NOT NULL,\n `x` INT NOT NULL,\n `z` INT NOT NULL,\n \
         `compression` SMALLINT UNSIGNED NOT NULL,\n `data` LONGBLOB NOT NULL,\n \
         PRIMARY KEY (`map`, `storage`, `x`, `z`),\n CONSTRAINT `fk_${prefix}grid_map`\n  FOREIGN KEY (`map`)\n  \
         REFERENCES `${prefix}map` (`id`)\n  ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n \
         CONSTRAINT `fk_${prefix}grid`\n  FOREIGN KEY (`storage`)\n  REFERENCES `${prefix}grid_storage` (`id`)\n  \
         ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n CONSTRAINT `fk_${prefix}grid_compression`\n  \
         FOREIGN KEY (`compression`)\n  REFERENCES `${prefix}compression` (`id`)\n  ON UPDATE RESTRICT\n  \
         ON DELETE CASCADE\n) COLLATE 'utf8mb4_bin'"
            .into(),
    ]
}

fn postgres() -> Vec<String> {
    let d = Dialect::Postgres;
    vec![
        key_table(d, "map", "SMALLSERIAL"),
        key_table(d, "compression", "SMALLSERIAL"),
        key_table(d, "item_storage", "SERIAL"),
        "CREATE TABLE IF NOT EXISTS ${prefix}item_storage_data (\n map SMALLINT NOT NULL\n  \
         REFERENCES ${prefix}map (id)\n  ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n storage INT NOT NULL\n  \
         REFERENCES ${prefix}item_storage (id)\n  ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n \
         compression SMALLINT NOT NULL\n  REFERENCES ${prefix}compression (id)\n  ON UPDATE RESTRICT\n  \
         ON DELETE CASCADE,\n data BYTEA NOT NULL,\n PRIMARY KEY (map, storage)\n)"
            .into(),
        key_table(d, "grid_storage", "SMALLSERIAL"),
        "CREATE TABLE IF NOT EXISTS ${prefix}grid_storage_data (\n map SMALLINT NOT NULL\n  \
         REFERENCES ${prefix}map (id)\n  ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n storage SMALLINT NOT NULL\n  \
         REFERENCES ${prefix}grid_storage (id)\n  ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n x INT NOT NULL,\n \
         z INT NOT NULL,\n compression SMALLINT NOT NULL\n  REFERENCES ${prefix}compression (id)\n  \
         ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n data BYTEA NOT NULL,\n PRIMARY KEY (map, storage, x, z)\n)"
            .into(),
    ]
}

fn sqlite() -> Vec<String> {
    let d = Dialect::Sqlite;
    vec![
        key_table(d, "map", ""),
        key_table(d, "compression", ""),
        key_table(d, "item_storage", ""),
        "CREATE TABLE IF NOT EXISTS `${prefix}item_storage_data` (\n `map` INTEGER NOT NULL,\n \
         `storage` INTEGER NOT NULL,\n `compression` INTEGER NOT NULL,\n `data` BLOB NOT NULL,\n \
         PRIMARY KEY (`map`, `storage`),\n CONSTRAINT `fk_${prefix}item_map`\n  FOREIGN KEY (`map`)\n  \
         REFERENCES `${prefix}map` (`id`)\n  ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n \
         CONSTRAINT `fk_${prefix}item`\n  FOREIGN KEY (`storage`)\n  REFERENCES `${prefix}item_storage` (`id`)\n  \
         ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n CONSTRAINT `fk_${prefix}item_compression`\n  \
         FOREIGN KEY (`compression`)\n  REFERENCES `${prefix}compression` (`id`)\n  ON UPDATE RESTRICT\n  \
         ON DELETE CASCADE\n) STRICT"
            .into(),
        key_table(d, "grid_storage", ""),
        "CREATE TABLE IF NOT EXISTS `${prefix}grid_storage_data` (\n `map` INTEGER NOT NULL,\n \
         `storage` INTEGER NOT NULL,\n `x` INTEGER NOT NULL,\n `z` INTEGER NOT NULL,\n \
         `compression` INTEGER NOT NULL,\n `data` BLOB NOT NULL,\n PRIMARY KEY (`map`, `storage`, `x`, `z`),\n \
         CONSTRAINT `fk_${prefix}grid_map`\n  FOREIGN KEY (`map`)\n  REFERENCES `${prefix}map` (`id`)\n  \
         ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n CONSTRAINT `fk_${prefix}grid`\n  FOREIGN KEY (`storage`)\n  \
         REFERENCES `${prefix}grid_storage` (`id`)\n  ON UPDATE RESTRICT\n  ON DELETE CASCADE,\n \
         CONSTRAINT `fk_${prefix}grid_compression`\n  FOREIGN KEY (`compression`)\n  \
         REFERENCES `${prefix}compression` (`id`)\n  ON UPDATE RESTRICT\n  ON DELETE CASCADE\n) STRICT"
            .into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_matches_upstream_text() {
        let sql = create_tables(Dialect::MySql, "bluemap_");
        assert_eq!(
            sql[0],
            "CREATE TABLE IF NOT EXISTS `bluemap_map` (\n `id` SMALLINT UNSIGNED NOT NULL AUTO_INCREMENT,\n \
             `map_id` VARCHAR(190) NOT NULL,\n PRIMARY KEY (`id`),\n UNIQUE INDEX `map_id` (`map_id`)\n) COLLATE 'utf8mb4_bin'"
        );
        assert!(sql[2].contains("`id` INT UNSIGNED NOT NULL AUTO_INCREMENT"));
        assert!(sql[5].contains("CONSTRAINT `fk_bluemap_grid_compression`"));
        assert!(sql.iter().all(|s| !s.contains("${prefix}")));
    }

    #[test]
    fn postgres_and_sqlite_types() {
        let pg = create_tables(Dialect::Postgres, "p_");
        assert_eq!(
            pg[0],
            "CREATE TABLE IF NOT EXISTS p_map (\n id SMALLSERIAL PRIMARY KEY,\n map_id VARCHAR(190) UNIQUE NOT NULL\n)"
        );
        assert!(pg[2].contains("id SERIAL PRIMARY KEY"));
        assert!(pg[5].contains("data BYTEA NOT NULL,\n PRIMARY KEY (map, storage, x, z)"));
        let lite = create_tables(Dialect::Sqlite, "");
        assert_eq!(
            lite[1],
            "CREATE TABLE IF NOT EXISTS `compression` (\n `id` INTEGER PRIMARY KEY AUTOINCREMENT,\n `key` TEXT UNIQUE NOT NULL\n) STRICT"
        );
        assert!(lite[3].ends_with(") STRICT"));
    }
}
