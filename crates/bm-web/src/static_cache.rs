//! Webroot lookups kept in memory so conditional and repeat requests skip the filesystem. A disk result is
//! re-checked with one `metadata` once it is older than [`REVALIDATE`] (bundled files only change if a disk file
//! appears in front of them); bodies up to [`MAX_BODY`] are kept, with a gzip copy built on first demand.

use std::collections::HashMap;
use std::fs::Metadata;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use bytes::Bytes;
use http::HeaderValue;

use crate::http_date::java_http_date;
use crate::paths::join;
use crate::webapp::{embedded_file, embedded_is_dir};

const REVALIDATE: Duration = Duration::from_secs(1);
const MAX_BODY: u64 = 8 << 20;
/// Bounds for the whole cache; crossing either drops every entry (gzip copies are not counted, they're smaller).
const MAX_ENTRIES: usize = 4096;
const MAX_BYTES: usize = 128 << 20;
const MIN_GZIP: usize = 512;
const GZIP_LEVEL: u32 = 9;

pub(crate) enum Content {
    Memory(Bytes),
    /// Too big to keep: streamed from this path per request.
    Disk(PathBuf),
}

pub(crate) struct FileEntry {
    pub len: u64,
    pub mtime_ms: i64,
    pub content: Content,
    pub last_modified: Option<HeaderValue>,
    embedded: bool,
    gzip: OnceLock<Option<Bytes>>,
}

impl FileEntry {
    fn new(len: u64, mtime_ms: i64, content: Content, embedded: bool) -> Arc<Self> {
        let last_modified = (mtime_ms > 0).then(|| HeaderValue::from_str(&java_http_date(mtime_ms)).ok()).flatten();
        Arc::new(Self { len, mtime_ms, content, last_modified, embedded, gzip: OnceLock::new() })
    }

    fn memory_len(&self) -> usize {
        match &self.content {
            Content::Memory(b) => b.len(),
            Content::Disk(_) => 0,
        }
    }

    /// The gzip copy if it is worth sending, without computing it.
    pub fn gzip_ready(&self) -> Option<Option<Bytes>> {
        self.gzip.get().cloned()
    }

    /// The gzip copy if it is worth sending. CPU-bound the first time: call from a blocking context.
    pub fn gzip(&self) -> Option<Bytes> {
        self.gzip
            .get_or_init(|| match &self.content {
                Content::Memory(data) if data.len() >= MIN_GZIP => {
                    let gz = bm_compress::gzip_with_level(data, GZIP_LEVEL);
                    (gz.len() < data.len() - data.len() / 16).then(|| gz.into())
                }
                _ => None,
            })
            .clone()
    }
}

/// What a webroot-relative path is: a directory (on disk or bundled) and/or a servable file.
#[derive(Clone, Default)]
pub(crate) struct Node {
    pub is_dir: bool,
    pub file: Option<Arc<FileEntry>>,
}

struct Slot {
    node: Node,
    checked: Instant,
}

#[derive(Default)]
struct Slots {
    map: HashMap<String, Slot>,
    bytes: usize,
}

pub(crate) struct StaticCache {
    root: Arc<Path>,
    embedded: bool,
    slots: RwLock<Slots>,
}

impl StaticCache {
    pub fn new(root: &Path, embedded: bool) -> Self {
        Self { root: root.into(), embedded, slots: RwLock::default() }
    }

    /// `rel` is normalized and `/`-separated. At most one blocking-pool hop, none while the entry is fresh.
    pub async fn lookup(&self, rel: &str) -> Node {
        let prev = {
            let slots = self.slots.read().unwrap_or_else(|e| e.into_inner());
            match slots.map.get(rel) {
                Some(slot) if slot.checked.elapsed() < REVALIDATE => return slot.node.clone(),
                slot => slot.map(|s| s.node.clone()),
            }
        };
        let (root, embedded, key) = (self.root.clone(), self.embedded, rel.to_owned());
        let node = tokio::task::spawn_blocking(move || resolve(&root, embedded, &key, prev)).await.unwrap_or_default();
        self.store(rel, node.clone());
        node
    }

    fn store(&self, rel: &str, node: Node) {
        let added = node.file.as_ref().map_or(0, |f| f.memory_len());
        let mut slots = self.slots.write().unwrap_or_else(|e| e.into_inner());
        if slots.map.len() >= MAX_ENTRIES || slots.bytes + added > MAX_BYTES {
            *slots = Slots::default();
        }
        if let Some(old) = slots.map.insert(rel.to_owned(), Slot { node, checked: Instant::now() }) {
            slots.bytes -= old.node.file.map_or(0, |f| f.memory_len());
        }
        slots.bytes += added;
    }
}

/// Disk first (a directory there hides a bundled file of the same name), then the bundled webapp; `prev` is
/// reused while the file it describes is unchanged.
fn resolve(root: &Path, embedded: bool, rel: &str, prev: Option<Node>) -> Node {
    let prev = prev.and_then(|n| n.file);
    let embedded_dir = || embedded && embedded_is_dir(rel);
    let path = join(root, rel);
    match std::fs::metadata(&path) {
        Ok(m) if m.is_dir() => Node { is_dir: true, file: None },
        Ok(m) => {
            let (len, mtime_ms) = (m.len(), mtime_ms(&m));
            let file = match prev {
                Some(f) if !f.embedded && f.len == len && f.mtime_ms == mtime_ms => Some(f),
                _ => load_disk(path, len, mtime_ms),
            };
            Node { is_dir: embedded_dir(), file }
        }
        Err(_) => {
            let file = match prev {
                Some(f) if f.embedded => Some(f),
                _ => embedded.then(|| embedded_file(rel)).flatten().map(|e| {
                    FileEntry::new(e.data.len() as u64, e.last_modified_ms, Content::Memory(e.data), true)
                }),
            };
            Node { is_dir: embedded_dir(), file }
        }
    }
}

fn load_disk(path: PathBuf, len: u64, mtime_ms: i64) -> Option<Arc<FileEntry>> {
    let mut file = std::fs::File::open(&path).ok()?;
    if len > MAX_BODY {
        return Some(FileEntry::new(len, mtime_ms, Content::Disk(path), false));
    }
    let mut data = Vec::with_capacity(len as usize);
    file.read_to_end(&mut data).ok()?;
    // a file rewritten between metadata and read keeps its old mtime here; the next revalidation reloads it
    Some(FileEntry::new(data.len() as u64, mtime_ms, Content::Memory(data.into()), false))
}

fn mtime_ms(m: &Metadata) -> i64 {
    let ms = m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis());
    i64::try_from(ms).unwrap_or(0)
}
