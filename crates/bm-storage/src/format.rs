//! Storage formats and how an existing storage reveals its own.
//!
//! - `compat`: upstream BlueMap's layout, byte-compatible (Java BlueMap, nginx `gzip_static`, `sql.php`).
//! - `optimized`: as compat except hires tiles, which are BMQ3 blobs ([`bm_format::compact`]): packed in
//!   bundles in a file storage ([`crate::optimized`]), one row per tile in SQL. Served only by bluemap-rs.
//!
//! A storage records `optimized` with a marker (file: [`FILE_MARKER`] in the storage root; SQL: the
//! [`SQL_MARKER`] row in `grid_storage`). No marker + any data = compat. Opening never changes a storage's format:
//! a configured format that disagrees with the stored one is refused with [`Error::FormatMismatch`]
//! (refusing beats guessing: reading compat as optimized would show an empty map, and writing would mix
//! layouts). An empty storage opened as optimized gets the marker. Conversion: [`crate::convert_file_storage`],
//! [`crate::convert_sql_storage`].

use std::fmt;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    #[default]
    Compat,
    Optimized,
}

impl Format {
    pub fn id(self) -> &'static str {
        match self {
            Self::Compat => "compat",
            Self::Optimized => "optimized",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Compat, Self::Optimized].into_iter().find(|f| f.id().eq_ignore_ascii_case(id))
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// File in the storage root of an optimized file storage.
pub const FILE_MARKER: &str = "bluemap-rs-format.txt";
/// Contents of [`FILE_MARKER`]: format name and layout version.
pub(crate) const FILE_MARKER_CONTENT: &[u8] = b"optimized 1\n";
/// `grid_storage.key` of the optimized marker row (it never has data rows).
pub const SQL_MARKER: &str = "bluemap-rs:format/optimized-1";

/// What a storage holds: `None` when it is empty (or missing).
pub(crate) fn check(location: &str, configured: Format, found: Option<Format>) -> Result<()> {
    match found {
        Some(found) if found != configured => {
            Err(Error::FormatMismatch { location: location.to_owned(), configured, found })
        }
        _ => Ok(()),
    }
}
