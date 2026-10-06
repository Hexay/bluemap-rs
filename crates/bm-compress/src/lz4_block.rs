//! lz4-java's `LZ4BlockOutputStream` framing (Minecraft chunk compression 4, BlueMap's `lz4` storage), not the
//! standard LZ4 frame format. Blocks of: "LZ4Block", token (method | log2(block size) - 10), compressed len,
//! original len, checksum (i32 LE); an empty block ends the stream.
//! <https://github.com/lz4/lz4-java/blob/master/src/java/net/jpountz/lz4/LZ4BlockInputStream.java>

use twox_hash::XxHash32;

use crate::{Error, Result};

pub const MAGIC: &[u8; 8] = b"LZ4Block";
pub const HEADER: usize = MAGIC.len() + 1 + 4 + 4 + 4;
pub const RAW: u8 = 0x10;
pub const LZ4: u8 = 0x20;
const SEED: u32 = 0x9747_b28c;
/// lz4-java's default block size, 64 KiB.
const LEVEL: u8 = 6;
const BLOCK: usize = 1 << (10 + LEVEL);

/// lz4-java's `StreamingXXHash32.asChecksum()` keeps only the low 28 bits.
pub fn checksum(block: &[u8]) -> u32 {
    XxHash32::oneshot(SEED, block) & 0x0FFF_FFFF
}

fn header(out: &mut Vec<u8>, method: u8, compressed: usize, original: usize, check: u32) {
    out.extend_from_slice(MAGIC);
    out.push(method | LEVEL);
    for v in [compressed as u32, original as u32, check] {
        out.extend(v.to_le_bytes());
    }
}

fn corrupt(msg: impl Into<String>) -> Error {
    Error::Lz4(msg.into())
}

/// Appends what lz4-java writes: 64 KiB blocks, each LZ4 unless that doesn't shrink it, then the end block.
/// Decodable by lz4-java, though not byte-identical to its JNI compressor.
pub fn compress_into(data: &[u8], out: &mut Vec<u8>) {
    for block in data.chunks(BLOCK) {
        let packed = lz4_flex::block::compress(block);
        let (method, payload) = if packed.len() < block.len() { (LZ4, &packed[..]) } else { (RAW, block) };
        header(out, method, payload.len(), block.len(), checksum(block));
        out.extend_from_slice(payload);
    }
    header(out, RAW, 0, 0, 0);
}

/// Appends blocks up to the empty end block, or the end of input (lz4-java tolerates a missing end block).
pub fn decompress_into(mut data: &[u8], limit: usize, out: &mut Vec<u8>) -> Result<()> {
    let base = out.len();
    while !data.is_empty() {
        if data.len() < HEADER {
            return Err(corrupt("truncated block header"));
        }
        if data[..MAGIC.len()] != *MAGIC {
            return Err(corrupt("bad block magic"));
        }
        let token = data[8];
        let le = |i: usize| i32::from_le_bytes(data[i..i + 4].try_into().unwrap());
        let (compressed, original, check) = (le(9), le(13), le(17) as u32);
        let method = token & 0xF0;
        if method != RAW && method != LZ4 {
            return Err(corrupt(format!("unknown block method {method:#x}")));
        }
        let (Ok(compressed), Ok(original)) = (usize::try_from(compressed), usize::try_from(original)) else {
            return Err(corrupt(format!("negative block length {compressed}/{original}")));
        };
        if original == 0 && compressed == 0 {
            if check != 0 {
                return Err(corrupt("corrupt end block"));
            }
            break;
        }
        // LZ4 sequences expand at most ~255x, so this rejects absurd declared lengths before allocating
        let plausible = original > 0
            && compressed > 0
            && original <= 1 << (10 + (token & 0x0F))
            && original <= compressed.saturating_mul(256)
            && (method == LZ4 || compressed == original);
        if !plausible {
            return Err(corrupt(format!("bad block lengths {compressed}/{original}")));
        }
        if out.len() - base + original > limit {
            return Err(Error::TooLarge { limit });
        }
        let payload = data.get(HEADER..HEADER + compressed).ok_or_else(|| corrupt("truncated block payload"))?;
        let start = out.len();
        if method == RAW {
            out.extend_from_slice(payload);
        } else {
            out.resize(start + original, 0);
            let n = lz4_flex::block::decompress_into(payload, &mut out[start..]).map_err(|e| corrupt(e.to_string()))?;
            if n != original {
                return Err(corrupt(format!("block decompressed to {n} bytes, header says {original}")));
            }
        }
        if checksum(&out[start..]) != check {
            return Err(corrupt("block checksum mismatch"));
        }
        data = &data[HEADER + compressed..];
    }
    Ok(())
}

#[cfg(test)]
#[path = "lz4_block_tests.rs"]
mod tests;
