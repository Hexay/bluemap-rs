//! [`Version`] of a stored file: mtime, length and file id. Every write is a new file renamed over the old one
//! ([`super::fsops::write_atomic`], Java's `.filepart` move too), so the id changes even when two writes land in
//! the same mtime tick with the same length (seen on NTFS at ~1 ms per write).

use std::fs::File;
use std::io::{self, ErrorKind, Read};
use std::path::Path;

use bm_compress::Compression;

use super::fsops::retry;
use crate::api::{Stored, Version};
use crate::error::{IoContext, Result};

/// `None` when missing (or a directory, which reads as missing too).
pub(crate) fn file_version(path: &Path) -> Result<Option<Version>> {
    missing_as_none(path, retry(path, || of_handle(&File::open(path)?)))
}

/// The file's bytes and the version of exactly those bytes (one handle), like [`super::fsops::read`].
pub(crate) fn read_versioned(path: &Path) -> Result<Option<(Vec<u8>, Version)>> {
    let read = retry(path, || {
        let mut file = File::open(path)?;
        let Some(version) = of_handle(&file)? else { return Ok(None) };
        let mut data = Vec::with_capacity(usize::try_from(version.0[1]).unwrap_or(0));
        file.read_to_end(&mut data)?;
        Ok(Some((data, version)))
    });
    missing_as_none(path, read)
}

pub(crate) fn read_stored(path: &Path, compression: Compression) -> Result<Option<(Stored, Option<Version>)>> {
    Ok(read_versioned(path)?.map(|(data, v)| (Stored { data, compression }, Some(v))))
}

fn missing_as_none<T>(path: &Path, r: io::Result<Option<T>>) -> Result<Option<T>> {
    match r {
        Ok(v) => Ok(v),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        // Windows can't open a directory as a file
        Err(_) if path.is_dir() => Ok(None),
        Err(e) => Err(e).ctx("read", path),
    }
}

/// `None` for a directory.
#[cfg(windows)]
fn of_handle(file: &File) -> io::Result<Option<Version>> {
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DIRECTORY, GetFileInformationByHandle,
    };

    // SAFETY: the handle is open for the call and `info` is a plain out-struct
    let info = unsafe {
        let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        if GetFileInformationByHandle(file.as_raw_handle(), &mut info) == 0 {
            return Err(io::Error::last_os_error());
        }
        info
    };
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        return Ok(None);
    }
    let wide = |hi: u32, lo: u32| u64::from(hi) << 32 | u64::from(lo);
    let mtime = wide(info.ftLastWriteTime.dwHighDateTime, info.ftLastWriteTime.dwLowDateTime);
    let len = wide(info.nFileSizeHigh, info.nFileSizeLow);
    Ok(Some(Version([mtime, len, wide(info.nFileIndexHigh, info.nFileIndexLow)])))
}

#[cfg(not(windows))]
fn of_handle(file: &File) -> io::Result<Option<Version>> {
    let m = file.metadata()?;
    if !m.is_file() {
        return Ok(None);
    }
    let mtime = m.modified()?.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    #[cfg(unix)]
    let id = std::os::unix::fs::MetadataExt::ino(&m);
    #[cfg(not(unix))]
    let id = 0;
    Ok(Some(Version([mtime, m.len(), id])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::fsops::write_atomic;

    #[test]
    fn every_write_gets_a_new_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t");
        assert_eq!(file_version(&path).unwrap(), None);
        assert_eq!(file_version(dir.path()).unwrap(), None);
        assert_eq!(read_versioned(dir.path()).unwrap(), None);
        let mut prev = None;
        for _ in 0..200 {
            write_atomic(&path, b"same bytes").unwrap();
            let v = file_version(&path).unwrap();
            assert!(v.is_some());
            assert_eq!(read_versioned(&path).unwrap(), Some((b"same bytes".to_vec(), v.unwrap())));
            assert_ne!(v, prev, "a rewrite kept its version");
            prev = v;
        }
    }
}
