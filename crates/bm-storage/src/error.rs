use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{op} {path}: {source}")]
    Io { op: &'static str, path: PathBuf, source: io::Error },
    #[error("sql: {0}")]
    Sql(#[from] sqlx::Error),
    #[error(transparent)]
    Compression(#[from] bm_compress::Error),
    #[error("storage is read-only")]
    ReadOnly,
    #[error("invalid map id {0:?}")]
    InvalidMapId(String),
    #[error("invalid table prefix {0:?}: must match [a-z0-9_]{{0,32}}")]
    InvalidTablePrefix(String),
    #[error(
        "unsupported sql url {0:?}: expected mysql:, mariadb:, postgres(ql): or sqlite: (optionally jdbc:-prefixed)"
    )]
    UnsupportedUrl(String),
    #[error("{size} byte blob exceeds the MySQL server's max_allowed_packet ({limit}): raise it or enable compression")]
    BlobTooLarge { size: usize, limit: usize },
    #[error("read-only database lacks BlueMap tables: {0}")]
    MissingTables(String),
    #[error("unexpected sql result: {0}")]
    Protocol(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) trait IoContext<T> {
    fn ctx(self, op: &'static str, path: &std::path::Path) -> Result<T>;
}

impl<T> IoContext<T> for io::Result<T> {
    fn ctx(self, op: &'static str, path: &std::path::Path) -> Result<T> {
        self.map_err(|source| Error::Io { op, path: path.to_path_buf(), source })
    }
}
