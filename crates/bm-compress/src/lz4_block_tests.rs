use super::*;

const LIMIT: usize = 1 << 20;

fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    compress_into(data, &mut out);
    out
}

fn decompress(data: &[u8], limit: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    decompress_into(data, limit, &mut out).map(|()| out)
}

/// 150 KB over three 64 KiB blocks: compressible, incompressible (stored raw), compressible.
fn sample() -> Vec<u8> {
    let mut x = 0x9e37_79b9u32;
    (0..150_000u32)
        .map(|i| {
            if (65536..131072).contains(&i) {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            } else {
                (i / 3 % 251) as u8
            }
        })
        .collect()
}

fn set_le(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

#[test]
fn round_trip_mixes_lz4_and_raw_blocks() {
    let data = sample();
    let stream = compress(&data);
    let second = HEADER + u32::from_le_bytes(stream[9..13].try_into().unwrap()) as usize;
    assert_eq!((stream[8] & 0xF0, stream[second + 8] & 0xF0), (LZ4, RAW));
    assert_eq!(decompress(&stream, LIMIT).unwrap(), data);
}

#[test]
fn missing_end_block_is_tolerated() {
    let data = sample();
    let stream = compress(&data);
    assert_eq!(decompress(&stream[..stream.len() - HEADER], LIMIT).unwrap(), data);
}

#[test]
fn empty_input_round_trips() {
    assert_eq!(compress(&[]).len(), HEADER);
    assert!(decompress(&compress(&[]), LIMIT).unwrap().is_empty());
}

#[test]
fn corrupt_streams_are_errors() {
    let stream = compress(&sample());
    type Case = (&'static str, fn(&mut Vec<u8>), &'static str);
    let cases: [Case; 8] = [
        ("magic", |s| s[3] = b'X', "bad block magic"),
        ("header", |s| s.truncate(HEADER - 1), "truncated block header"),
        ("payload", |s| s.truncate(100), "truncated block payload"),
        ("checksum", |s| s[17] ^= 1, "checksum mismatch"),
        ("method", |s| s[8] = 0x30 | 6, "unknown block method"),
        ("over_level", |s| set_le(s, 13, 65537), "bad block lengths"),
        ("over_ratio", |s| set_le(s, 9, 65536 / 256 - 1), "bad block lengths"),
        ("negative", |s| set_le(s, 9, u32::MAX), "negative block length"),
    ];
    for (name, corrupt, want) in cases {
        let mut s = stream.clone();
        corrupt(&mut s);
        let e = decompress(&s, LIMIT).unwrap_err().to_string();
        assert!(e.contains(want), "{name}: {e}");
    }
}

#[test]
fn declared_length_mismatches_are_errors() {
    let mut s = compress(&[7; 9]);
    assert_eq!(s[8] & 0xF0, RAW);
    set_le(&mut s, 13, 8);
    assert!(decompress(&s, LIMIT).unwrap_err().to_string().contains("bad block lengths 9/8"));

    let mut s = compress(&[1; 1000]);
    set_le(&mut s, 13, 1001);
    let e = decompress(&s, LIMIT).unwrap_err().to_string();
    assert!(e.contains("decompressed to 1000 bytes, header says 1001"), "{e}");
    set_le(&mut s, 13, 999);
    assert!(decompress(&s, LIMIT).unwrap_err().to_string().contains("lz4 block stream: "));
}

#[test]
fn output_over_limit_is_refused() {
    let e = decompress(&compress(&[0; 5000]), 4096).unwrap_err().to_string();
    assert!(e.contains("exceeds 4096 bytes"), "{e}");
}

#[test]
fn appends_after_existing_output() {
    let mut out = b"keep".to_vec();
    decompress_into(&compress(b"tile"), LIMIT, &mut out).unwrap();
    assert_eq!(out, b"keeptile");
}
