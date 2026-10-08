use super::*;

/// What a hires tile starts with (PRBM format version) followed by filler.
const TILE: &[u8] = b"\x01\x07prbm tile prbm tile prbm tile prbm tile";
const LIMIT: usize = 1 << 20;
const COMPRESSED: [Compression; 4] = [Compression::Gzip, Compression::Deflate, Compression::Zstd, Compression::Lz4];

#[test]
fn every_compression_round_trips_and_is_detected() {
    for c in Compression::ALL {
        let bytes = c.compress(TILE).unwrap();
        assert_eq!(Compression::detect(&bytes), c);
        assert_eq!(c.decompress(&bytes, LIMIT).unwrap(), TILE, "{c:?}");
    }
}

#[test]
fn uncompressed_bodies_are_detected_as_none() {
    for body in [TILE, b"{\"maps\":[]}", b"[]", b"\x89PNG\r\n\x1a\n"] {
        assert_eq!(Compression::detect(body), Compression::None);
    }
}

#[test]
fn ids_keys_and_suffixes_match_bluemap() {
    let table: Vec<_> = Compression::ALL.iter().map(|c| (c.id(), c.key(), c.file_suffix())).collect();
    assert_eq!(
        table,
        [
            ("none", "bluemap:none", ""),
            ("gzip", "bluemap:gzip", ".gz"),
            ("deflate", "bluemap:deflate", ".deflate"),
            ("zstd", "bluemap:zstd", ".zst"),
            ("lz4", "bluemap:lz4", ".lz4"),
        ]
    );
    assert_eq!(Compression::from_id("bluemap:zstd"), Some(Compression::Zstd));
    assert_eq!(Compression::from_id("gzip"), Some(Compression::Gzip));
    assert_eq!(Compression::from_id("brotli"), None);
}

#[test]
fn chunk_types_map_to_compressions() {
    let types: Vec<_> = (0..=5).chain([127]).map(Compression::from_chunk_type).collect();
    let c = |c| Some(c);
    let want =
        [None, c(Compression::Gzip), c(Compression::Deflate), c(Compression::None), c(Compression::Lz4), None, None];
    assert_eq!(types, want);
}

#[test]
fn concatenated_gzip_members_are_all_read() {
    let mut bytes = Compression::Gzip.compress(b"first ").unwrap();
    bytes.extend(Compression::Gzip.compress(b"second").unwrap());
    assert_eq!(Compression::Gzip.decompress(&bytes, LIMIT).unwrap(), b"first second");
}

#[test]
fn output_over_limit_is_refused() {
    let big = vec![0; 10_000];
    for c in Compression::ALL {
        let e = c.decompress(&c.compress(&big).unwrap(), 4096).unwrap_err();
        assert!(matches!(e, Error::TooLarge { limit: 4096 }), "{c:?}: {e}");
    }
}

#[test]
fn corrupt_input_is_an_error() {
    for c in COMPRESSED {
        let mut bytes = c.compress(TILE).unwrap();
        bytes.truncate(bytes.len() / 2);
        assert!(c.decompress(&bytes, LIMIT).is_err(), "{c:?}");
    }
}

#[test]
fn incompressible_input_outgrows_the_first_output_guess() {
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let noise: Vec<u8> = (0..200_000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 32) as u8
        })
        .collect();
    for c in [Compression::Gzip, Compression::Deflate] {
        let packed = c.compress(&noise).unwrap();
        assert!(packed.len() > first_output_guess(noise.len()), "{c:?}: the retry path must run");
        assert_eq!(c.decompress(&packed, LIMIT).unwrap(), noise, "{c:?}");
    }
}

#[test]
fn into_variants_replace_buffer_contents() {
    let mut buf = b"stale".to_vec();
    for c in COMPRESSED {
        c.compress_into(TILE, &mut buf).unwrap();
        let packed = buf.clone();
        c.decompress_into(&packed, LIMIT, &mut buf).unwrap();
        assert_eq!(buf, TILE, "{c:?}");
    }
}
