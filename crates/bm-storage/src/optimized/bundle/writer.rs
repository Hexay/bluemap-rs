//! The append path of [`BundleStore`]: held write handles, torn-tail repair, compaction and removal.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::Path;

use bm_format::grid::Tile;

use super::log::{self, Index, Local};
use super::{Bundle, BundleStore, MAX_WRITERS, lock, split};
use crate::error::{Error, IoContext, Result};
use crate::file::fsops;
use crate::key::GridKey;
use crate::locks::LockKey;

/// Compaction threshold: dead bytes must exceed both the live bytes and this.
const COMPACT_MIN_DEAD: u64 = 1 << 20;

/// What an append leaves to do with the bundle file once its write handle is closed.
enum After {
    Keep,
    Remove,
    Replace(Vec<u8>, Index),
}

impl BundleStore {
    /// Appends a put (`Some`) or delete record under the bundle's writer lock.
    pub(super) fn append(&self, tile: Tile, data: Option<&[u8]>) -> Result<()> {
        if self.read_only {
            return Err(Error::ReadOnly);
        }
        let (bundle, local) = split(tile);
        let path = self.path(bundle);
        let _writer = self.writers.lock(LockKey::Grid(GridKey::Hires, bundle));
        let state = self.bundle(bundle);
        let mut b = lock(&state);
        let opened = b.writer.is_none();
        if opened && data.is_none() && !path.exists() {
            return Ok(());
        }
        let after = Self::append_locked(&path, &mut b, local, data).ctx("write", &path);
        let after = match after {
            Ok(after) => after,
            Err(e) => {
                // the in-memory index may be ahead of the file now: reload it from the file next time
                *b = Bundle::default();
                return Err(e);
            }
        };
        match after {
            After::Keep => {}
            After::Remove => {
                *b = Bundle::default();
                fsops::remove_file(&path)?;
            }
            After::Replace(image, next) => {
                b.writer = None;
                b.ix = Index::default();
                fsops::write_atomic(&path, &image)?;
                b.ix = next;
            }
        }
        let holds = b.writer.is_some();
        drop(b);
        if opened && holds {
            self.track_writer(bundle);
        }
        Ok(())
    }

    fn append_locked(path: &Path, b: &mut Bundle, local: Local, data: Option<&[u8]>) -> io::Result<After> {
        if b.writer.is_none() {
            b.writer = Some(Self::open_writer(path, &mut b.ix)?);
        }
        if data.is_none() && !b.ix.entries.contains_key(&local) {
            return Ok(After::Keep);
        }
        let mut buf = Vec::with_capacity(data.map_or(0, <[u8]>::len) + 32);
        let write_at = if b.ix.generation == 0 {
            b.ix = Index::fresh(log::new_generation());
            buf.extend(log::header(b.ix.generation));
            0
        } else {
            b.ix.end
        };
        let payload = data.unwrap_or_default();
        log::record(&mut buf, data.is_some(), local, payload);
        let w = b.writer.as_mut().expect("writer opened above");
        w.seek(SeekFrom::Start(write_at))?;
        w.write_all(&buf)?;
        let end = b.ix.end;
        b.ix.apply(data.is_some(), local, end, payload.len() as u32);
        if b.ix.entries.is_empty() {
            return Ok(After::Remove);
        }
        if b.ix.dead() > b.ix.live.max(COMPACT_MIN_DEAD) {
            b.writer = None;
            let mut f = File::open(path)?;
            let (image, next) = log::compacted(&log::read_range(&mut f, 0, b.ix.end)?, &b.ix);
            return Ok(After::Replace(image, next));
        }
        Ok(After::Keep)
    }

    /// Loads the index through a read handle (never read through the write handle: see the module docs), then
    /// opens the write handle and cuts a torn tail or header.
    fn open_writer(path: &Path, ix: &mut Index) -> io::Result<File> {
        let len = match Self::open_read(path)? {
            Some(mut f) => Self::refresh(&mut f, ix)?,
            None => {
                *ix = Index::default();
                0
            }
        };
        let open = || fsops::retry(path, || OpenOptions::new().write(true).create(true).truncate(false).open(path));
        let w = match open() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
                open()?
            }
            other => other?,
        };
        let valid = if ix.generation == 0 { 0 } else { ix.end };
        if len > valid {
            w.set_len(valid)?;
        }
        Ok(w)
    }

    /// Remembers a bundle holding a writer; closes the oldest beyond [`MAX_WRITERS`].
    fn track_writer(&self, bundle: Tile) {
        let evict = {
            let mut open = lock(&self.open);
            open.push_back(bundle);
            if open.len() > MAX_WRITERS { open.pop_front() } else { None }
        };
        // the state mutex alone guards the handle; taking the writer key lock here could deadlock
        if let Some(old) = evict.filter(|&old| old != bundle)
            && let Some(state) = lock(&self.cache).get(&old).cloned()
        {
            lock(&state).writer = None;
        }
    }
}
