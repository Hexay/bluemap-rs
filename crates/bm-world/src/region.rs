//! Anvil region files (`r.<x>.<z>.mca`): an 8 KiB header of chunk locations and timestamps, then 4 KiB sectors.
//! One open handle per region and positioned reads, unlike BlueMap's open/close per chunk (docs/01 §1).
//!
//! Area loads [`Region::preload`] their chunks first. Windows serialises reads on one handle, so per-chunk reads from
//! every render thread queue behind each other, and on a disk they seek back and forth (docs/12 "Round 6").

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use bm_compress::Compression;

use crate::{Error, Result};

const SECTOR: usize = 4096;
/// Decompressed chunks are rarely over 1 MiB; this only stops corrupt lengths from exhausting memory.
const CHUNK_LIMIT: usize = 256 << 20;
/// Chunk type flag: the payload lives in an external `c.<x>.<z>.mcc` file.
const EXTERNAL: u8 = 128;
/// Chunks this close in the file are preloaded with one read, gap included.
const PRELOAD_GAP: u64 = 256 << 10;

pub struct Region {
    file: Option<File>,
    dir: PathBuf,
    pub x: i32,
    pub z: i32,
    header: Box<[u8; 2 * SECTOR]>,
    len: u64,
    /// File offset and bytes of each [`Region::preload`] read, by offset.
    preloaded: Vec<(u64, Vec<u8>)>,
}

impl Region {
    /// `dir/r.<x>.<z>.mca`. A missing or empty file is an empty region, not an error.
    pub fn open(dir: &Path, x: i32, z: i32) -> Result<Self> {
        let mut header = Box::new([0; 2 * SECTOR]);
        let mut len = 0;
        let file = match File::open(dir.join(format!("r.{x}.{z}.mca"))) {
            Ok(file) => {
                let read = read_at_most(&file, &mut header[..], 0)?;
                len = file.metadata()?.len();
                (read > 0).then_some(file)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        Ok(Self { file, dir: dir.to_owned(), x, z, header, len, preloaded: Vec::new() })
    }

    /// Reads the sectors of the local chunks `chunks` now, neighbours in the file with one read, so
    /// [`Region::read_chunk_into`] serves them from memory. A read that fails is left to that call to report.
    pub fn preload(&mut self, chunks: impl Iterator<Item = (usize, usize)>) {
        let Some(file) = &self.file else { return };
        let mut spans: Vec<(u64, u64)> = chunks
            .map(|(lx, lz)| self.location(lx, lz))
            .filter(|&(offset, size)| offset != 0 && size != 0 && offset < self.len)
            .map(|(offset, size)| (offset, (offset + size as u64).min(self.len)))
            .collect();
        spans.sort_unstable();
        self.preloaded.clear();
        let mut spans = spans.into_iter().peekable();
        while let Some((start, mut end)) = spans.next() {
            while let Some(&(_, next_end)) = spans.peek().filter(|next| next.0 <= end + PRELOAD_GAP) {
                end = end.max(next_end);
                spans.next();
            }
            let mut buf = vec![0; (end - start) as usize];
            match read_at_most(file, &mut buf, start) {
                Ok(n) if n == buf.len() => self.preloaded.push((start, buf)),
                _ => return,
            }
        }
    }

    /// The chunk's sectors as far as the file goes, if a [`Region::preload`] read holds them.
    fn preloaded(&self, offset: u64, size: usize) -> Option<&[u8]> {
        let i = self.preloaded.partition_point(|run| run.0 <= offset).checked_sub(1)?;
        let (start, buf) = &self.preloaded[i];
        let (at, run_end) = ((offset - start) as usize, start + buf.len() as u64);
        let end = at.checked_add(size)?;
        // a chunk that wasn't preloaded may start in a run and continue past it
        (end <= buf.len() || (at < buf.len() && run_end == self.len)).then(|| &buf[at..end.min(buf.len())])
    }

    fn location(&self, lx: usize, lz: usize) -> (u64, usize) {
        let i = 4 * (lz * 32 + lx);
        let h = &self.header[i..i + 4];
        let offset = u32::from_be_bytes([0, h[0], h[1], h[2]]) as u64 * SECTOR as u64;
        (offset, h[3] as usize * SECTOR)
    }

    /// Last save time (unix seconds) of local chunk `lx, lz` (0..32), 0 if never written. BlueMap's change detection
    /// compares these.
    pub fn timestamp(&self, lx: usize, lz: usize) -> u32 {
        let i = SECTOR + 4 * (lz * 32 + lx);
        u32::from_be_bytes(self.header[i..i + 4].try_into().unwrap())
    }

    /// The timestamp of a chunk with sectors allocated, as `MCARegion.iterateAllChunks` reports them (signed, like
    /// Java's `int`); `None` for chunks it skips.
    pub fn listed_timestamp(&self, lx: usize, lz: usize) -> Option<i32> {
        (self.file.is_some() && self.location(lx, lz).1 > 0).then(|| self.timestamp(lx, lz) as i32)
    }

    pub fn has_chunk(&self, lx: usize, lz: usize) -> bool {
        self.file.is_some() && self.location(lx, lz).0 != 0
    }

    /// Decompressed chunk NBT into `out`; `Ok(false)` if the chunk was never generated. `raw` is scratch.
    pub fn read_chunk_into(&self, lx: usize, lz: usize, raw: &mut Vec<u8>, out: &mut Vec<u8>) -> Result<bool> {
        let (offset, size) = self.location(lx, lz);
        let Some(file) = self.file.as_ref().filter(|_| offset != 0 && size != 0) else {
            return Ok(false);
        };
        let sectors = match self.preloaded(offset, size) {
            Some(sectors) => sectors,
            None => {
                raw.resize(size, 0);
                let n = read_at_most(file, raw, offset)?;
                &raw[..n]
            }
        };
        if sectors.len() < 5 {
            return Err(Error::Corrupt("chunk sector past the end of the region file"));
        }
        // the 4-byte length is ignored, like BlueMap does: some writers leave it wrong
        let ty = sectors[4];
        let external;
        let (ty, payload) = if ty & EXTERNAL != 0 {
            let (cx, cz) = (self.x * 32 + lx as i32, self.z * 32 + lz as i32);
            external = std::fs::read(self.dir.join(format!("c.{cx}.{cz}.mcc")))?;
            (ty & !EXTERNAL, &external[..])
        } else {
            (ty, &sectors[5..])
        };
        // 0 isn't a vanilla type; BlueMap reads it as uncompressed
        let compression = if ty == 0 { Some(Compression::None) } else { Compression::from_chunk_type(ty) };
        let compression = compression.ok_or(Error::UnsupportedCompression(ty))?;
        compression.decompress_into(payload, CHUNK_LIMIT, out)?;
        Ok(true)
    }
}

/// `r.<x>.<z>.mca` → region coordinates; BlueMap ignores regions beyond ±100000.
pub fn parse_region_file_name(name: &str) -> Option<(i32, i32)> {
    let rest = name.strip_prefix("r.")?.strip_suffix(".mca")?;
    let (x, z) = rest.split_once('.')?;
    let (x, z) = (x.parse::<i32>().ok()?, z.parse::<i32>().ok()?);
    (x.abs() <= 100_000 && z.abs() <= 100_000).then_some((x, z))
}

/// Region coordinates of the non-empty `.mca` files in `dir`.
pub fn list_regions(dir: &Path) -> Result<Vec<(i32, i32)>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let mut regions = Vec::new();
    for entry in entries {
        let entry = entry?;
        let coords = entry.file_name().to_str().and_then(parse_region_file_name);
        if let Some(coords) = coords.filter(|_| entry.metadata().is_ok_and(|m| m.len() > 0)) {
            regions.push(coords);
        }
    }
    regions.sort_unstable();
    Ok(regions)
}

/// Fills `buf` from `offset` as far as the file goes; returns the bytes read.
fn read_at_most(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    let mut done = 0;
    while done < buf.len() {
        match read_at(file, &mut buf[done..], offset + done as u64) {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(done)
}

#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A region with uncompressed chunks `(lx, lz, first sector, payload)`, cut to `len` bytes if given.
    fn region(dir: &Path, chunks: &[(usize, usize, u8, &[u8])], len: Option<u64>) -> Region {
        let mut file = vec![0u8; 2 * SECTOR];
        for &(lx, lz, sector, payload) in chunks {
            let sectors = (payload.len() + 5).div_ceil(SECTOR);
            file[4 * (lz * 32 + lx)..][..4].copy_from_slice(&[0, 0, sector, sectors as u8]);
            file.resize(file.len().max((sector as usize + sectors) * SECTOR), 0);
            let at = sector as usize * SECTOR;
            file[at + 4] = 3;
            file[at + 5..at + 5 + payload.len()].copy_from_slice(payload);
        }
        file.truncate(len.map_or(file.len(), |l| l as usize));
        std::fs::write(dir.join("r.0.0.mca"), file).unwrap();
        Region::open(dir, 0, 0).unwrap()
    }

    fn read(region: &Region, lx: usize, lz: usize) -> std::result::Result<Option<Vec<u8>>, String> {
        let (mut raw, mut out) = (Vec::new(), Vec::new());
        let found = region.read_chunk_into(lx, lz, &mut raw, &mut out).map_err(|e| e.to_string())?;
        Ok(found.then_some(out))
    }

    #[test]
    fn preloaded_chunks_read_like_direct_ones() {
        let dir = tempfile::tempdir().unwrap();
        let big = vec![7u8; 3 * SECTOR];
        // sectors: a 2, b 3..=6, c 200 (beyond the gap from b), d 201
        let chunks: [(usize, usize, u8, &[u8]); 4] =
            [(0, 0, 2, b"a"), (1, 0, 3, &big), (2, 0, 200, b"c"), (3, 0, 201, b"d")];
        let all = [(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)];
        // whole, cut inside d's sector, cut inside b, cut before c
        for len in [None, Some(201 * SECTOR as u64 + 3), Some(5 * SECTOR as u64 + 9), Some(7 * SECTOR as u64)] {
            let direct = region(dir.path(), &chunks, len);
            let want: Vec<_> = all.iter().map(|&(lx, lz)| read(&direct, lx, lz)).collect();
            for picked in [&all[..], &all[..2], &all[2..3], &[(1, 0), (3, 0)]] {
                let mut preloaded = region(dir.path(), &chunks, len);
                preloaded.preload(picked.iter().copied());
                let got: Vec<_> = all.iter().map(|&(lx, lz)| read(&preloaded, lx, lz)).collect();
                assert_eq!(got, want, "len {len:?}, preloaded {picked:?}");
            }
        }
        let mut whole = region(dir.path(), &chunks, None);
        whole.preload(all.into_iter());
        assert_eq!(
            whole.preloaded.iter().map(|(start, buf)| (*start, buf.len())).collect::<Vec<_>>(),
            [(2 * SECTOR as u64, 5 * SECTOR), (200 * SECTOR as u64, 2 * SECTOR)]
        );
    }
}
