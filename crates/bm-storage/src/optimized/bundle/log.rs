//! Bundle file layout (spec in [`super`]): header, checksummed records, index scan, compaction image.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fs::File;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicU64, Ordering};

use twox_hash::XxHash32;

const MAGIC: &[u8; 4] = b"BMB1";
pub(super) const HEADER_LEN: u64 = 16;
/// Tiles per bundle side: `1 << SHIFT`.
pub(super) const SHIFT: u32 = 4;
const RECORD_HEAD: usize = 12;
const PUT: u8 = 1;
const DELETE: u8 = 2;

/// Bundle-local tile coordinates (`0..16` each).
pub(super) type Local = (u8, u8);

/// In-memory index of one bundle file version, rebuilt from the file whenever it may be stale.
#[derive(Debug, Default)]
pub(super) struct Index {
    /// Random id of the file version; 0 = no file (or not scanned yet).
    pub generation: u64,
    /// End of the last valid record (where the next append goes).
    pub end: u64,
    /// Offset of the record and its payload length per live tile.
    pub entries: HashMap<Local, (u64, u32)>,
    /// Bytes of the live records (headers included).
    pub live: u64,
}

impl Index {
    pub fn fresh(generation: u64) -> Self {
        Self { generation, end: HEADER_LEN, ..Self::default() }
    }

    pub fn dead(&self) -> u64 {
        self.end.saturating_sub(HEADER_LEN + self.live)
    }

    /// Accounts for the record at `offset` (a put, or a delete when `!put`).
    pub fn apply(&mut self, put: bool, local: Local, offset: u64, len: u32) {
        if let Some((_, old)) = self.entries.remove(&local) {
            self.live -= RECORD_HEAD as u64 + u64::from(old);
        }
        if put {
            self.entries.insert(local, (offset, len));
            self.live += RECORD_HEAD as u64 + u64::from(len);
        }
        self.end = offset + RECORD_HEAD as u64 + u64::from(len);
    }
}

pub(super) fn new_generation() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(SEQ.fetch_add(1, Ordering::Relaxed));
    h.write_u32(std::process::id());
    h.finish() | 1
}

pub(super) fn header(generation: u64) -> [u8; HEADER_LEN as usize] {
    let mut h = [0; HEADER_LEN as usize];
    h[..4].copy_from_slice(MAGIC);
    h[4..8].copy_from_slice(&SHIFT.to_le_bytes());
    h[8..].copy_from_slice(&generation.to_le_bytes());
    h
}

/// Generation of the file behind `f`; `Ok(None)` if it is shorter than a header (new or torn on creation).
pub(super) fn read_generation(f: &mut File, len: u64) -> io::Result<Option<u64>> {
    if len < HEADER_LEN {
        return Ok(None);
    }
    let mut h = [0; HEADER_LEN as usize];
    f.seek(SeekFrom::Start(0))?;
    f.read_exact(&mut h)?;
    if &h[..4] != MAGIC || h[4..8] != SHIFT.to_le_bytes() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a BMB1 tile bundle"));
    }
    Ok(Some(u64::from_le_bytes(h[8..].try_into().unwrap())))
}

fn checksum(head: &[u8], data: &[u8]) -> u32 {
    let mut h = XxHash32::with_seed(0);
    h.write(&head[..8]);
    h.write(data);
    h.finish_32()
}

/// Appends one record (`u8 kind, u8 lx, u8 lz, u8 0, u32 len, u32 xxh32(head[..8] ‖ data)`, data) to `out`.
pub(super) fn record(out: &mut Vec<u8>, put: bool, (lx, lz): Local, data: &[u8]) {
    let mut head = [0u8; RECORD_HEAD];
    head[..4].copy_from_slice(&[if put { PUT } else { DELETE }, lx, lz, 0]);
    head[4..8].copy_from_slice(&(data.len() as u32).to_le_bytes());
    let sum = checksum(&head, data);
    head[8..].copy_from_slice(&sum.to_le_bytes());
    out.extend(head);
    out.extend(data);
}

/// Parses the valid records in `buf` (file bytes from `index.end`) into `index`; stops at the first torn one.
pub(super) fn scan(buf: &[u8], index: &mut Index) {
    let base = index.end;
    let mut at = 0;
    while let Some(head) = buf.get(at..at + RECORD_HEAD) {
        let len = u32::from_le_bytes(head[4..8].try_into().unwrap());
        let Some(data) = buf.get(at + RECORD_HEAD..at + RECORD_HEAD + len as usize) else { break };
        let valid = matches!(head[0], PUT | DELETE) && head[1] < 16 && head[2] < 16 && head[3] == 0;
        if !valid || checksum(head, data) != u32::from_le_bytes(head[8..].try_into().unwrap()) {
            break;
        }
        index.apply(head[0] == PUT, (head[1], head[2]), base + at as u64, len);
        at += RECORD_HEAD + len as usize;
    }
}

/// Reads `[from, to)` of the file.
pub(super) fn read_range(f: &mut File, from: u64, to: u64) -> io::Result<Vec<u8>> {
    let mut buf = vec![0; usize::try_from(to - from).map_err(|_| io::ErrorKind::InvalidData)?];
    f.seek(SeekFrom::Start(from))?;
    f.read_exact(&mut buf)?;
    Ok(buf)
}

/// Payload of the record at `offset` if it is the intact PUT of `local` (else `None`: the index is wrong).
pub(super) fn read_record(f: &mut File, offset: u64, local: Local, len: u32) -> io::Result<Option<Vec<u8>>> {
    let buf = read_range(f, offset, offset + RECORD_HEAD as u64 + u64::from(len))?;
    let (head, data) = buf.split_at(RECORD_HEAD);
    let ok = head[..4] == [PUT, local.0, local.1, 0]
        && head[4..8] == len.to_le_bytes()
        && checksum(head, data) == u32::from_le_bytes(head[8..].try_into().unwrap());
    Ok(ok.then(|| data.to_vec()))
}

/// A new file image holding only the live records of `index` (from `file`, the bytes of the indexed version),
/// with a new generation, and its index.
pub(super) fn compacted(file: &[u8], index: &Index) -> (Vec<u8>, Index) {
    let generation = new_generation();
    let mut out = Vec::with_capacity((HEADER_LEN + index.live) as usize);
    out.extend(header(generation));
    let mut next = Index::fresh(generation);
    let mut live: Vec<_> = index.entries.iter().collect();
    live.sort_unstable_by_key(|(local, _)| **local);
    for (&local, &(offset, len)) in live {
        let start = (offset as usize) + RECORD_HEAD;
        next.apply(true, local, out.len() as u64, len);
        record(&mut out, true, local, &file[start..start + len as usize]);
    }
    (out, next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_stops_at_torn_records_and_tracks_garbage() {
        let mut buf = Vec::new();
        record(&mut buf, true, (1, 2), b"first");
        record(&mut buf, true, (1, 2), b"second");
        record(&mut buf, true, (3, 4), b"x");
        record(&mut buf, false, (3, 4), b"");
        let whole = buf.len();
        record(&mut buf, true, (5, 5), b"torn");
        buf.truncate(buf.len() - 1);
        let mut index = Index::fresh(7);
        scan(&buf, &mut index);
        assert_eq!(index.end, HEADER_LEN + whole as u64);
        assert_eq!(index.entries.len(), 1);
        assert_eq!(index.entries[&(1, 2)], (HEADER_LEN + 17, 6));
        assert_eq!(index.live, 18);
        let mut file = header(7).to_vec();
        file.extend(&buf);
        let (image, next) = compacted(&file, &index);
        assert_eq!(image.len() as u64, HEADER_LEN + 18);
        assert_eq!(next.entries[&(1, 2)], (HEADER_LEN, 6));
        assert_eq!(next.dead(), 0);
    }
}
