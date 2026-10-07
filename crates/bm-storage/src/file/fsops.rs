//! Filesystem primitives that survive Windows: atomic replace, transient-lock retries, read-only attributes.
//!
//! Upstream writes `<file>.filepart` and moves it over the target, which fails on Windows when an antivirus,
//! indexer or a concurrent reader briefly holds the target (#471, #241, #666), deletes hit the same (#669), and
//! two writers sharing one `.filepart` name corrupt each other (#821). Here every write uses a unique temp file in
//! the target's directory, `std::fs::rename` (MoveFileExW REPLACE_EXISTING / POSIX-semantics rename), and
//! retries sharing/lock violations with backoff. A failed write (e.g. disk full) removes its temp file and never
//! touches the old target.

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::error::{IoContext, Result};

pub(crate) const TEMP_SUFFIX: &str = ".filepart";

const BACKOFF_MS: [u64; 10] = [1, 2, 5, 10, 20, 50, 100, 200, 400, 800];

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Errors that a scanner or another process holding the file causes for a moment.
fn is_transient(e: &io::Error) -> bool {
    // ACCESS_DENIED (also "delete pending"), SHARING_VIOLATION, LOCK_VIOLATION
    cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

pub(crate) fn retry<T>(path: &Path, mut op: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut cleared_readonly = false;
    for delay in BACKOFF_MS {
        match op() {
            Err(e) if is_transient(&e) => {
                if !cleared_readonly && e.kind() == ErrorKind::PermissionDenied {
                    cleared_readonly = true;
                    clear_readonly(path);
                }
                std::thread::sleep(Duration::from_millis(delay));
            }
            other => return other,
        }
    }
    op()
}

fn clear_readonly(path: &Path) {
    if let Ok(meta) = fs::symlink_metadata(path) {
        let mut perms = meta.permissions();
        if perms.readonly() && meta.is_file() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = fs::set_permissions(path, perms);
        }
    }
}

/// `None` when missing.
pub(crate) fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    match retry(path, || fs::read(path)) {
        Ok(data) => Ok(Some(data)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        // a directory where a file is expected reads as missing, like Java's Files.exists + open
        Err(_) if path.is_dir() => Ok(None),
        Err(e) => Err(e).ctx("read", path),
    }
}

/// Atomically replaces `path` with `data`, creating parent directories.
pub(crate) fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp = temp_path(path);
    let written = create_temp(dir, &tmp).and_then(|mut f| {
        f.write_all(data)?;
        // the handle must be closed before the rename on Windows
        drop(f);
        retry(path, || {
            fs::rename(&tmp, path).map_err(|e| {
                // Windows reports a directory target as ACCESS_DENIED; don't retry that
                if path.is_dir() { io::Error::new(ErrorKind::IsADirectory, e) } else { e }
            })
        })
    });
    if written.is_err() {
        let _ = retry(&tmp, || fs::remove_file(&tmp));
    }
    written.ctx("write", path)
}

fn create_temp(dir: &Path, tmp: &Path) -> io::Result<File> {
    let open = || retry(tmp, || OpenOptions::new().write(true).create_new(true).open(tmp));
    match open() {
        Err(e) if e.kind() == ErrorKind::NotFound => {
            retry(dir, || fs::create_dir_all(dir))?;
            open()
        }
        other => other,
    }
}

/// Unique per process and write, so concurrent writers of one key never share a temp file.
fn temp_path(path: &Path) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{:x}-{seq:x}{TEMP_SUFFIX}", std::process::id()));
    path.with_file_name(name)
}

/// Removes a file; missing is fine.
pub(crate) fn remove_file(path: &Path) -> Result<()> {
    match retry(path, || fs::remove_file(path)) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e).ctx("delete", path),
        _ => Ok(()),
    }
}

/// Removes a file or a whole directory tree; missing is fine. Entries vanishing concurrently are ignored.
pub(crate) fn remove_tree(path: &Path) -> Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).ctx("stat", path),
    };
    if !meta.is_dir() {
        return remove_file(path);
    }
    match fs::read_dir(path) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry.ctx("list", path)?;
                remove_tree(&entry.path())?;
            }
        }
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).ctx("list", path),
    }
    match retry(path, || fs::remove_dir(path)) {
        Err(e) if e.kind() != ErrorKind::NotFound => Err(e).ctx("delete", path),
        _ => Ok(()),
    }
}

/// Paths of all regular files under `dir` (recursive); a missing dir yields nothing.
pub(crate) fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).ctx("list", dir),
    };
    for entry in entries {
        let entry = entry.ctx("list", dir)?;
        let kind = entry.file_type().ctx("stat", &entry.path())?;
        if kind.is_dir() {
            walk_files(&entry.path(), out)?;
        } else if kind.is_file() {
            out.push(entry.path());
        }
    }
    Ok(())
}

/// Entries up to `depth` levels below `dir`, parents before children (Java `FileHelper.walk(root, 3)` order).
pub(crate) fn walk_depth(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<()> {
    if depth == 0 {
        return Ok(());
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).ctx("list", dir),
    };
    for entry in entries {
        let entry = entry.ctx("list", dir)?;
        out.push(entry.path());
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            walk_depth(&entry.path(), depth - 1, out)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a/b/c.bin");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"two");
        assert_eq!(fs::read_dir(file.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn failed_write_keeps_old_target_and_cleans_temp() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("t");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"x").unwrap();
        assert!(write_atomic(&target, b"new").is_err());
        assert_eq!(fs::read(target.join("keep")).unwrap(), b"x");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn readonly_files_are_replaced_and_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ro");
        fs::write(&file, b"old").unwrap();
        let mut perms = fs::metadata(&file).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&file, perms).unwrap();
        write_atomic(&file, b"new").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"new");
        let mut perms = fs::metadata(&file).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&file, perms).unwrap();
        remove_tree(dir.path()).unwrap();
        assert!(!dir.path().exists());
    }

    #[test]
    fn missing_paths_are_not_errors() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read(&dir.path().join("nope")).unwrap(), None);
        remove_file(&dir.path().join("nope")).unwrap();
        remove_tree(&dir.path().join("nope")).unwrap();
        let mut v = Vec::new();
        walk_files(&dir.path().join("nope"), &mut v).unwrap();
        assert!(v.is_empty());
    }
}
