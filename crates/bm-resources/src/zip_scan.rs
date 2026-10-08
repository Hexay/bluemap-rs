//! A fast path for reading one entry of a big zip: `zip::ZipArchive::new` indexes every entry (~100 ms for a
//! 34k-entry client jar), while finding one name in the raw central directory takes a few ms.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const EOCD_SIG: u32 = 0x0605_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const LOCAL_SIG: u32 = 0x0403_4b50;

/// The entry's text when the zip is plain (no zip64, no prefix data, the name listed exactly once, stored or
/// deflated, valid UTF-8); `None` otherwise, and the caller falls back to the `zip` crate.
pub(crate) fn entry_string(zip: &Path, name: &str) -> Option<String> {
    let mut file = File::open(zip).ok()?;
    let len = file.metadata().ok()?.len();
    let tail_len = len.min(22 + 0xFFFF);
    let tail = read_at(&mut file, len - tail_len, tail_len as usize)?;
    let eocd = (0..=tail.len().checked_sub(22)?).rev().find(|&i| u32_at(&tail, i) == EOCD_SIG)?;
    let (entries, cd_size, cd_offset) =
        (u16_at(&tail, eocd + 10), u32_at(&tail, eocd + 12), u32_at(&tail, eocd + 16));
    if entries == 0xFFFF || cd_size == u32::MAX || cd_offset == u32::MAX {
        return None;
    }
    if u64::from(cd_offset) + u64::from(cd_size) != len - tail_len + eocd as u64 {
        return None;
    }
    let cd = read_at(&mut file, u64::from(cd_offset), cd_size as usize)?;
    let (mut pos, mut found) = (0, None);
    for _ in 0..entries {
        if u32_at(&cd, pos) != CENTRAL_SIG {
            return None;
        }
        let (n, e, c) = (u16_at(&cd, pos + 28) as usize, u16_at(&cd, pos + 30) as usize, u16_at(&cd, pos + 32) as usize);
        if cd.get(pos + 46..pos + 46 + n)? == name.as_bytes() {
            if found.is_some() {
                return None;
            }
            found = Some((u16_at(&cd, pos + 10), u32_at(&cd, pos + 20), u32_at(&cd, pos + 24), u32_at(&cd, pos + 42)));
        }
        pos += 46 + n + e + c;
    }
    let (method, packed, size, local) = found?;
    if packed == u32::MAX || size == u32::MAX || local == u32::MAX {
        return None;
    }
    let header = read_at(&mut file, u64::from(local), 30)?;
    if u32_at(&header, 0) != LOCAL_SIG {
        return None;
    }
    let data_at = u64::from(local) + 30 + u64::from(u16_at(&header, 26)) + u64::from(u16_at(&header, 28));
    let data = read_at(&mut file, data_at, packed as usize)?;
    let bytes = match method {
        0 => data,
        8 => {
            let mut out = Vec::with_capacity(size as usize);
            flate2::read::DeflateDecoder::new(&data[..]).read_to_end(&mut out).ok()?;
            out
        }
        _ => return None,
    };
    (bytes.len() == size as usize).then(|| String::from_utf8(bytes).ok()).flatten()
}

fn read_at(file: &mut File, offset: u64, len: usize) -> Option<Vec<u8>> {
    file.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0; len];
    file.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    b.get(i..i + 2).map_or(0, |s| u16::from_le_bytes([s[0], s[1]]))
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    b.get(i..i + 4).map_or(0, |s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use super::entry_string;

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("bm-zip-scan-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn zip_bytes(entries: &[(&str, &str, zip::CompressionMethod)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (name, body, method) in entries {
            w.start_file(*name, zip::write::SimpleFileOptions::default().compression_method(*method)).unwrap();
            w.write_all(body.as_bytes()).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn reads_stored_and_deflated_entries() {
        let body = r#"{"pack_version": {"resource_major": 75}}"#.repeat(20);
        let zip = temp_file(
            "entries.zip",
            &zip_bytes(&[
                ("a.txt", "x", zip::CompressionMethod::Stored),
                ("version.json", &body, zip::CompressionMethod::Deflated),
                ("b.txt", "y", zip::CompressionMethod::Stored),
            ]),
        );
        assert_eq!(entry_string(&zip, "version.json").as_deref(), Some(&body[..]));
        assert_eq!(entry_string(&zip, "b.txt").as_deref(), Some("y"));
        assert_eq!(entry_string(&zip, "missing.json"), None);
        std::fs::remove_file(zip).unwrap();
    }

    #[test]
    fn prefixed_or_broken_zips_fall_back() {
        let stored = zip_bytes(&[("version.json", "{}", zip::CompressionMethod::Stored)]);
        let prefixed = temp_file("prefixed.zip", &[b"#!/bin/sh\n".as_slice(), &stored].concat());
        let broken = temp_file("broken.zip", b"not a zip at all");
        assert_eq!(entry_string(&prefixed, "version.json"), None);
        assert_eq!(entry_string(&broken, "version.json"), None);
        std::fs::remove_file(prefixed).unwrap();
        std::fs::remove_file(broken).unwrap();
    }
}
