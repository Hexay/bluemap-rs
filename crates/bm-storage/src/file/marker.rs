//! The optimized marker of a file storage root (rules in [`crate::format`]).

use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

use super::fsops;
use crate::error::{IoContext, Result};
use crate::format::{FILE_MARKER, FILE_MARKER_CONTENT, Format};

/// `Some(Optimized)` with a marker, `Some(Compat)` when anything else is stored, `None` when empty or missing.
pub(crate) fn detect_format(root: &Path) -> Result<Option<Format>> {
    let marker = root.join(FILE_MARKER);
    if let Some(content) = fsops::read(&marker)? {
        if content != FILE_MARKER_CONTENT {
            let e = io::Error::new(ErrorKind::InvalidData, "unknown storage format marker (newer bluemap-rs?)");
            return Err(e).ctx("read", &marker);
        }
        return Ok(Some(Format::Optimized));
    }
    Ok(any_file(root).ctx("list", root)?.then_some(Format::Compat))
}

/// Writes (`optimized`) or removes the marker.
pub(crate) fn set_marker(root: &Path, optimized: bool) -> Result<()> {
    let marker = root.join(FILE_MARKER);
    if optimized { fsops::write_atomic(&marker, FILE_MARKER_CONTENT) } else { fsops::remove_file(&marker) }
}

/// Stops at the first file found.
fn any_file(dir: &Path) -> io::Result<bool> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if any_file(&entry.path())? {
                return Ok(true);
            }
        } else {
            return Ok(true);
        }
    }
    Ok(false)
}
