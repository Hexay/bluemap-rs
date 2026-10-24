//! Sidecar index `x…z….bmi` of a bundle (spec in [`super`]): lets a reader open a bundle with one 2 KB read instead
//! of scanning the whole log.

use std::fs;
use std::hash::Hasher;
use std::path::Path;

use twox_hash::XxHash32;

use super::log::{HEADER_LEN, Index, RECORD_HEAD, SHIFT};

const MAGIC: &[u8; 4] = b"BMI1";
const SLOTS: usize = 1 << (2 * SHIFT);
const LEN: usize = 24 + SLOTS * 8 + 4;

fn checksum(data: &[u8]) -> u32 {
    let mut h = XxHash32::with_seed(0);
    h.write(data);
    h.finish_32()
}

/// The sidecar of `ix`; `None` when it can't describe it (no file version, or an offset past 4 GiB).
pub(super) fn encode(ix: &Index) -> Option<Vec<u8>> {
    if ix.generation == 0 {
        return None;
    }
    let mut slots = [(0u32, 0u32); SLOTS];
    for (&(lx, lz), &(offset, len)) in &ix.entries {
        slots[usize::from(lz) << SHIFT | usize::from(lx)] = (u32::try_from(offset).ok()?, len);
    }
    let mut out = Vec::with_capacity(LEN);
    out.extend(MAGIC);
    out.extend(SHIFT.to_le_bytes());
    out.extend(ix.generation.to_le_bytes());
    out.extend(ix.end.to_le_bytes());
    for (offset, len) in slots {
        out.extend(offset.to_le_bytes());
        out.extend(len.to_le_bytes());
    }
    out.extend(checksum(&out).to_le_bytes());
    Some(out)
}

/// The index in the sidecar at `path` if it describes file version `generation`, now `len` bytes long; any doubt
/// (missing, torn, other version, entries outside the covered length) is `None`.
pub(super) fn load(path: &Path, generation: u64, len: u64) -> Option<Index> {
    let bytes = fs::read(path).ok()?;
    let (body, sum) = bytes.split_at_checked(LEN - 4).filter(|(_, s)| s.len() == 4)?;
    let word = |at: usize| u32::from_le_bytes(body[at..at + 4].try_into().unwrap());
    let long = |at: usize| u64::from_le_bytes(body[at..at + 8].try_into().unwrap());
    if &body[..4] != MAGIC || word(4) != SHIFT || checksum(body) != u32::from_le_bytes(sum.try_into().unwrap()) {
        return None;
    }
    let covered = long(16);
    if long(8) != generation || covered > len || covered < HEADER_LEN {
        return None;
    }
    let mut ix = Index { sidecar: true, ..Index::fresh(generation) };
    ix.end = covered;
    for slot in 0..SLOTS {
        let (offset, size) = (u64::from(word(24 + slot * 8)), word(28 + slot * 8));
        if offset == 0 {
            continue;
        }
        let record = RECORD_HEAD as u64 + u64::from(size);
        if offset < HEADER_LEN || offset + record > covered {
            return None;
        }
        ix.entries.insert(((slot & 15) as u8, (slot >> SHIFT) as u8), (offset, size));
        ix.live += record;
    }
    Some(ix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_doubt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x0z0.bmi");
        let mut ix = Index::fresh(9);
        ix.apply(true, (3, 15), HEADER_LEN, 5);
        ix.apply(true, (15, 0), HEADER_LEN + 17, 1);
        let bytes = encode(&ix).unwrap();
        assert_eq!(bytes.len(), LEN);
        fs::write(&path, &bytes).unwrap();
        let back = load(&path, 9, ix.end).unwrap();
        assert_eq!((back.entries, back.end, back.live, back.sidecar), (ix.entries.clone(), ix.end, ix.live, true));
        assert!(load(&path, 8, ix.end).is_none(), "other file version");
        assert!(load(&path, 9, ix.end - 1).is_none(), "file shorter than covered");
        let mut torn = bytes.clone();
        torn[40] ^= 1;
        fs::write(&path, &torn).unwrap();
        assert!(load(&path, 9, ix.end).is_none(), "checksum");
        fs::write(&path, &bytes[..LEN - 1]).unwrap();
        assert!(load(&path, 9, ix.end).is_none(), "short");
        assert!(load(&dir.path().join("none.bmi"), 9, ix.end).is_none());
    }
}
