//! `storages/<id>.conf` (`StorageConfig.java`, `FileConfig.java`, `SQLConfig.java`) plus our `format` key.

use std::path::PathBuf;

use serde::Deserialize;

use crate::de::{DeError, fields, from_value};
use crate::key::Key;
use crate::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum StorageConfig {
    File(FileStorageConfig),
    Sql(SqlStorageConfig),
}

/// bluemap-rs only: on-disk layout of a storage. Missing means `compat`, so existing BlueMap storages stay
/// readable by Java BlueMap; new installs are generated with `optimized`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StorageFormat {
    #[default]
    Compat,
    Optimized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Deflate,
    Zstd,
    Lz4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Mysql,
    Mariadb,
    Postgresql,
    Sqlite,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct FileStorageConfig {
    pub root: PathBuf,
    /// Raw key; validated lazily by [`FileStorageConfig::compression`] like Java.
    pub compression: String,
    /// Hidden key.
    pub atomic: bool,
    #[serde(deserialize_with = "format")]
    pub format: StorageFormat,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct SqlStorageConfig {
    /// JDBC URL, e.g. `jdbc:mysql://localhost:3306/bluemap?permitMysqlScheme`.
    pub connection_url: String,
    #[serde(deserialize_with = "fields::string_map")]
    pub connection_properties: Vec<(String, String)>,
    /// `None`: derived from the connection URL ([`SqlStorageConfig::dialect`]).
    pub dialect: Option<String>,
    /// Java-only (JDBC driver jar); kept so configs round-trip.
    pub driver_jar: Option<String>,
    pub driver_class: Option<String>,
    /// < 0: unlimited.
    pub max_connections: i32,
    /// `None`: the dialect's defaults ([`SqlStorageConfig::connection_init_sql`]).
    pub connection_init_sql: Option<Vec<String>>,
    pub table_prefix: String,
    pub compression: String,
    #[serde(deserialize_with = "format")]
    pub format: StorageFormat,
}

impl Default for FileStorageConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("bluemap/web/maps"),
            compression: "bluemap:gzip".into(),
            atomic: true,
            format: StorageFormat::Compat,
        }
    }
}

impl Default for SqlStorageConfig {
    fn default() -> Self {
        Self {
            connection_url: "jdbc:mysql://localhost/bluemap?permitMysqlScheme".into(),
            connection_properties: Vec::new(),
            dialect: None,
            driver_jar: None,
            driver_class: None,
            max_connections: -1,
            connection_init_sql: None,
            table_prefix: "bluemap_".into(),
            compression: "bluemap:gzip".into(),
            format: StorageFormat::Compat,
        }
    }
}

fn format<'de, D: serde::Deserializer<'de>>(d: D) -> Result<StorageFormat, D::Error> {
    match String::deserialize(d)?.to_ascii_lowercase().as_str() {
        "compat" => Ok(StorageFormat::Compat),
        "optimized" => Ok(StorageFormat::Optimized),
        other => {
            Err(serde::de::Error::custom(format!("unknown storage format '{other}' (expected compat or optimized)")))
        }
    }
}

/// `StorageConfig.parseKey`: namespaced key (default `bluemap:`), falling back to the lower-cased legacy form.
fn parse_key<T: Copy>(registry: &[(&str, T)], raw: &str, what: &str) -> Result<T, String> {
    let find = |key: &Key| registry.iter().find(|(name, _)| *key == Key::bluemap(name)).map(|(_, t)| *t);
    find(&Key::parse_with_default(raw, Key::BLUEMAP))
        .or_else(|| find(&Key::bluemap(&raw.to_lowercase())))
        .ok_or_else(|| format!("no {what} found for key: {raw}"))
}

const COMPRESSIONS: &[(&str, Compression)] = &[
    ("none", Compression::None),
    ("gzip", Compression::Gzip),
    ("deflate", Compression::Deflate),
    ("zstd", Compression::Zstd),
    ("lz4", Compression::Lz4),
];

const DIALECTS: &[(&str, Dialect)] = &[
    ("mysql", Dialect::Mysql),
    ("mariadb", Dialect::Mariadb),
    ("postgresql", Dialect::Postgresql),
    ("sqlite", Dialect::Sqlite),
];

impl Dialect {
    pub fn key(self) -> &'static str {
        DIALECTS.iter().find(|(_, d)| *d == self).map(|(k, _)| *k).expect("every dialect is listed")
    }

    fn protocol(self) -> String {
        format!("jdbc:{}:", self.key())
    }

    /// `Dialect.getConnectionInitSql()`.
    pub fn default_init_sql(self) -> &'static [&'static str] {
        match self {
            Dialect::Mysql | Dialect::Mariadb => &[],
            Dialect::Postgresql => &["SET synchronous_commit = off"],
            Dialect::Sqlite => &[
                "PRAGMA journal_mode = WAL",
                "PRAGMA synchronous = NORMAL",
                "PRAGMA busy_timeout = 30000",
                "PRAGMA foreign_keys = ON",
            ],
        }
    }
}

impl FileStorageConfig {
    pub fn compression(&self) -> Result<Compression, String> {
        parse_key(COMPRESSIONS, &self.compression, "compression")
    }
}

impl SqlStorageConfig {
    pub fn compression(&self) -> Result<Compression, String> {
        parse_key(COMPRESSIONS, &self.compression, "compression")
    }

    pub fn dialect(&self) -> Result<Dialect, String> {
        match &self.dialect {
            Some(key) => parse_key(DIALECTS, key, "dialect"),
            None => DIALECTS.iter().map(|(_, d)| *d).find(|d| self.connection_url.starts_with(&d.protocol())).ok_or_else(|| {
                "could not find any sql-dialect matching the connection-url; check that it looks like jdbc:<dialect>://...".into()
            }),
        }
    }

    pub fn connection_init_sql(&self) -> Result<Vec<String>, String> {
        match &self.connection_init_sql {
            Some(sql) => Ok(sql.clone()),
            None => Ok(self.dialect()?.default_init_sql().iter().map(|s| s.to_string()).collect()),
        }
    }

    /// Up to 32 of `[a-z0-9_]`.
    pub fn table_prefix(&self) -> Result<&str, String> {
        let p = &self.table_prefix;
        let valid = p.len() <= 32 && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if valid {
            Ok(p)
        } else {
            Err(format!("the table-prefix '{p}' is invalid: only up to 32 of a-z, 0-9 and _ are allowed"))
        }
    }
}

#[derive(Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct Base {
    storage_type: String,
}

impl Default for Base {
    fn default() -> Self {
        Self { storage_type: "bluemap:file".into() }
    }
}

#[derive(Clone, Copy)]
enum StorageType {
    File,
    Sql,
}

impl StorageConfig {
    /// Loads the base config for `storage-type`, then the concrete type (BlueMap loads the file twice the same way).
    pub fn from_value(v: &Value) -> Result<StorageConfig, DeError> {
        let base: Base = from_value(v)?;
        let kind =
            parse_key(&[("file", StorageType::File), ("sql", StorageType::Sql)], &base.storage_type, "storage-type")
                .map_err(|message| DeError { path: vec!["storage-type".into()], message })?;
        Ok(match kind {
            StorageType::File => StorageConfig::File(from_value(v)?),
            StorageType::Sql => StorageConfig::Sql(from_value(v)?),
        })
    }

    pub fn format(&self) -> StorageFormat {
        match self {
            StorageConfig::File(c) => c.format,
            StorageConfig::Sql(c) => c.format,
        }
    }

    pub fn compression(&self) -> Result<Compression, String> {
        match self {
            StorageConfig::File(c) => c.compression(),
            StorageConfig::Sql(c) => c.compression(),
        }
    }
}
