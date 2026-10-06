//! Anvil region files (`r.<x>.<z>.mca`): an 8 KiB header of chunk locations and timestamps, then 4 KiB sectors.
//! One open handle per region and positioned reads, unlike BlueMap's open/close per chunk (docs/01 §1).

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

pub struct Region {
    file: Option<File>,
    dir: PathBuf,
    pub x: i32,
    pub z: i32,
    header: Box<[u8; 2 * SECTOR]>,
}

impl Region {
    /// `dir/r.<x>.<z>.mca`. A missing or empty file is an empty region, not an error.
    pub fn open(dir: &Path, x: i32, z: i32) -> Result<Self> {
        let mut header = Box::new([0; 2 * SECTOR]);
        let file = match File::open(dir.join(format!("r.{x}.{z}.mca"))) {
            Ok(file) => {
                let read = read_at_most(&file, &mut header[..], 0)?;
                (read > 0).then_some(file)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        Ok(Self { file, dir: dir.to_owned(), x, z, header })
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

    pub fn has_chunk(&self, lx: usize, lz: usize) -> bool {
        self.file.is_some() && self.location(lx, lz).0 != 0
    }

    /// Decompressed chunk NBT into `out`; `Ok(false)` if the chunk was never generated. `raw` is scratch.
    pub fn read_chunk_into(&self, lx: usize, lz: usize, raw: &mut Vec<u8>, out: &mut Vec<u8>) -> Result<bool> {
        let (offset, size) = self.location(lx, lz);
        let Some(file) = self.file.as_ref().filter(|_| offset != 0 && size != 0) else {
            return Ok(false);
        };
        raw.resize(size, 0);
        let n = read_at_most(file, raw, offset)?;
        if n < 5 {
            return Err(Error::Corrupt("chunk sector past the end of the region file"));
        }
        // the 4-byte length is ignored, like BlueMap does: some writers leave it wrong
        let ty = raw[4];
        let (ty, payload) = if ty & EXTERNAL != 0 {
            let (cx, cz) = (self.x * 32 + lx as i32, self.z * 32 + lz as i32);
            *raw = std::fs::read(self.dir.join(format!("c.{cx}.{cz}.mcc")))?;
            (ty & !EXTERNAL, &raw[..])
        } else {
            (ty, &raw[5..n])
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
