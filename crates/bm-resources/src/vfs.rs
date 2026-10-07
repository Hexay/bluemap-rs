//! A pack root as a read-only file tree: a folder, or a folder inside a zip/jar (also nested jars). Paths are
//! `/`-separated and relative to the root. Zips are held in memory and their archive handle cloned per read, so
//! packs can be read from many threads without a lock.

use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use zip::ZipArchive;

use crate::{Error, Result};

#[derive(Clone)]
pub struct Pack {
    source: Source,
    /// Folder inside the source this pack is rooted at, `""` or ending in `/`.
    prefix: String,
    /// For logs: the file or folder the pack came from.
    pub origin: Arc<str>,
}

#[derive(Clone)]
enum Source {
    Dir(PathBuf),
    Zip(Arc<ZipSource>),
}

struct ZipSource {
    archive: ZipArchive<Cursor<Arc<[u8]>>>,
    /// Every file entry, sorted, for prefix walks.
    files: BTreeSet<String>,
}

impl Pack {
    /// A folder, or a zip/jar file.
    pub fn open(path: &Path) -> Result<Self> {
        let origin: Arc<str> = path.display().to_string().into();
        if path.is_dir() {
            return Ok(Self { source: Source::Dir(path.to_owned()), prefix: String::new(), origin });
        }
        Self::zip(std::fs::read(path)?.into(), origin)
    }

    /// An in-memory zip, e.g. a jar nested in another pack.
    pub fn zip(bytes: Arc<[u8]>, origin: Arc<str>) -> Result<Self> {
        let archive = ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Pack(format!("{origin}: {e}")))?;
        let files = archive.file_names().filter(|n| !n.ends_with('/')).map(|n| n.replace('\\', "/")).collect();
        Ok(Self { source: Source::Zip(Arc::new(ZipSource { archive, files })), prefix: String::new(), origin })
    }

    /// This pack re-rooted at a subfolder (overlays, nested datapacks).
    pub fn sub(&self, dir: &str) -> Self {
        Self {
            source: self.source.clone(),
            prefix: format!("{}{}/", self.prefix, dir.trim_matches('/')),
            origin: self.origin.clone(),
        }
    }

    /// The subfolder this pack is rooted at inside its source, `""` or ending in `/`.
    pub fn root(&self) -> &str {
        &self.prefix
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let full = format!("{}{path}", self.prefix);
        match &self.source {
            Source::Dir(root) => std::fs::read(root.join(&full)).ok(),
            Source::Zip(z) => {
                let mut archive = z.archive.clone();
                let mut entry = archive.by_name(&full).ok()?;
                let mut buf = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buf).ok()?;
                Some(buf)
            }
        }
    }

    pub fn read_string(&self, path: &str) -> Option<String> {
        String::from_utf8(self.read(path)?).ok()
    }

    pub fn exists(&self, path: &str) -> bool {
        let full = format!("{}{path}", self.prefix);
        match &self.source {
            Source::Dir(root) => root.join(&full).exists(),
            Source::Zip(z) => z.files.contains(&full) || self.is_dir(path),
        }
    }

    pub fn is_dir(&self, path: &str) -> bool {
        let full = format!("{}{}/", self.prefix, path.trim_end_matches('/'));
        match &self.source {
            Source::Dir(root) => root.join(&full).is_dir(),
            Source::Zip(z) => z.files.range(full.clone()..).next().is_some_and(|f| f.starts_with(&full)),
        }
    }

    /// Names of the direct children (files and folders) of `dir`, sorted.
    pub fn list(&self, dir: &str) -> Vec<String> {
        let dir = dir.trim_matches('/');
        match &self.source {
            Source::Dir(root) => {
                let mut names: Vec<String> = std::fs::read_dir(root.join(&self.prefix).join(dir))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect();
                names.sort();
                names
            }
            Source::Zip(_) => {
                let base = if dir.is_empty() { String::new() } else { format!("{dir}/") };
                let names: BTreeSet<String> = self
                    .walk(dir)
                    .into_iter()
                    .filter_map(|f| f.strip_prefix(&base).and_then(|r| r.split('/').next()).map(str::to_owned))
                    .collect();
                names.into_iter().collect()
            }
        }
    }

    /// Every file under `dir`, recursively, as paths relative to the pack root, sorted.
    pub fn walk(&self, dir: &str) -> Vec<String> {
        let dir = dir.trim_matches('/');
        let base = if dir.is_empty() { self.prefix.clone() } else { format!("{}{dir}/", self.prefix) };
        match &self.source {
            Source::Dir(root) => {
                let mut out = Vec::new();
                walk_dir(&root.join(&base), &base[self.prefix.len()..], &mut out);
                out.sort();
                out
            }
            Source::Zip(z) => z
                .files
                .range(base.clone()..)
                .take_while(|f| f.starts_with(&base))
                .map(|f| f[self.prefix.len()..].to_owned())
                .collect(),
        }
    }
}

fn walk_dir(dir: &Path, rel: &str, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let Ok(name) = entry.file_name().into_string() else { continue };
        let path = format!("{rel}{name}");
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk_dir(&entry.path(), &format!("{path}/"), out),
            Ok(_) => out.push(path),
            Err(_) => {}
        }
    }
}

#[cfg(test)]
#[path = "vfs_tests.rs"]
mod tests;
