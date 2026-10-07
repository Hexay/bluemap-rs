//! `java.util.zip.Deflater` (madler zlib, `deflateInit2(level, Z_DEFLATED, 15, 8, Z_DEFAULT_STRATEGY)`) and
//! `java.util.zip.CRC32`.
//!
//! `IDATOutputStream` feeds the Deflater with `NO_FLUSH` and then `finish()`es; zlib's output doesn't depend on how
//! input and output are chunked under `NO_FLUSH`, so one `compress2` call gives the same bytes (checked against
//! every texture in BlueMap's `textures.json`).

use libz_sys as z;

/// The zlib stream of `data` at `level`, appended to `out`.
pub(super) fn deflate_into(data: &[u8], level: i32, out: &mut Vec<u8>) {
    let len = z::uLong::try_from(data.len()).expect("PNG pixel data exceeds zlib's length type");
    // SAFETY: plain C function on an integer
    let bound = unsafe { z::compressBound(len) } as usize;
    let start = out.len();
    out.resize(start + bound, 0);
    let mut written = bound as z::uLong;
    // SAFETY: `out[start..]` holds `bound` writable bytes and `data` is `len` readable bytes
    let rc = unsafe { z::compress2(out[start..].as_mut_ptr(), &mut written, data.as_ptr(), len, level) };
    // only Z_MEM_ERROR is possible with a compressBound-sized buffer and a valid level
    assert_eq!(rc, z::Z_OK, "zlib compress2 failed");
    out.truncate(start + written as usize);
}

pub(super) fn crc32(parts: &[&[u8]]) -> u32 {
    let mut crc: z::uLong = 0;
    for part in parts {
        for chunk in part.chunks(u32::MAX as usize) {
            // SAFETY: `chunk` is readable for its length, which fits `uInt`
            crc = unsafe { z::crc32(crc, chunk.as_ptr(), chunk.len() as z::uInt) };
        }
    }
    crc as u32
}
