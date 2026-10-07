//! Length-prefixed frames: `u32 frame_len | u32 header_len | header JSON | body` (big-endian, as Java's
//! `DataOutputStream` writes them).

use std::collections::HashMap;
use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

pub const MAX_HEADER: usize = 1 << 20;
pub const MAX_BODY: usize = 64 << 20;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("ipc i/o: {0}")]
    Io(#[from] io::Error),
    #[error("stream ended inside a frame")]
    Truncated,
    #[error("frame too large: header {header} B, body {body} B")]
    TooLarge { header: usize, body: usize },
    #[error("malformed frame length {0}")]
    BadLength(u32),
    #[error("malformed header: {0}")]
    Header(#[from] serde_json::Error),
    #[error("header has no string field \"t\"")]
    NoType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub header: serde_json::Value,
    pub body: Vec<u8>,
}

impl Frame {
    /// The `t` field.
    pub fn kind(&self) -> &str {
        self.header.get("t").and_then(|t| t.as_str()).unwrap_or_default()
    }

    pub fn parse<T: DeserializeOwned>(&self) -> Result<T, IpcError> {
        Ok(T::deserialize(&self.header)?)
    }
}

/// Reads one frame; `Ok(None)` on a clean EOF between frames.
pub fn read_frame(r: &mut impl Read) -> Result<Option<Frame>, IpcError> {
    let mut len = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match r.read(&mut len[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(IpcError::Truncated),
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
    }
    let frame_len = u32::from_be_bytes(len);
    if frame_len < 4 {
        return Err(IpcError::BadLength(frame_len));
    }
    let header_len = u32::from_be_bytes(read_array(r)?) as usize;
    let rest = frame_len as usize - 4;
    if header_len > rest {
        return Err(IpcError::BadLength(frame_len));
    }
    let body_len = rest - header_len;
    if header_len > MAX_HEADER || body_len > MAX_BODY {
        return Err(IpcError::TooLarge { header: header_len, body: body_len });
    }
    let header_bytes = read_vec(r, header_len)?;
    let body = read_vec(r, body_len)?;
    let header: serde_json::Value = serde_json::from_slice(&header_bytes)?;
    if !header.get("t").is_some_and(serde_json::Value::is_string) {
        return Err(IpcError::NoType);
    }
    Ok(Some(Frame { header, body }))
}

fn read_array<const N: usize>(r: &mut impl Read) -> Result<[u8; N], IpcError> {
    let mut buf = [0u8; N];
    r.read_exact(&mut buf).map_err(truncated)?;
    Ok(buf)
}

fn read_vec(r: &mut impl Read, len: usize) -> Result<Vec<u8>, IpcError> {
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).map_err(truncated)?;
    Ok(buf)
}

fn truncated(e: io::Error) -> IpcError {
    if e.kind() == io::ErrorKind::UnexpectedEof { IpcError::Truncated } else { IpcError::Io(e) }
}

/// Writes one frame with a single `write_all` and flushes.
pub fn write_frame(w: &mut impl Write, header: &impl Serialize, body: &[u8]) -> Result<(), IpcError> {
    let header = serde_json::to_vec(header)?;
    if header.len() > MAX_HEADER || body.len() > MAX_BODY {
        return Err(IpcError::TooLarge { header: header.len(), body: body.len() });
    }
    let mut buf = Vec::with_capacity(8 + header.len() + body.len());
    buf.extend_from_slice(&((4 + header.len() + body.len()) as u32).to_be_bytes());
    buf.extend_from_slice(&(header.len() as u32).to_be_bytes());
    buf.extend_from_slice(&header);
    buf.extend_from_slice(body);
    w.write_all(&buf)?;
    w.flush()?;
    Ok(())
}

/// Joins `Markers` continuation frames per map.
#[derive(Default)]
pub struct MarkerAssembler {
    partial: HashMap<String, Vec<u8>>,
}

impl MarkerAssembler {
    /// The complete JSON once the last part (`more == false`) of `map` arrived.
    pub fn push(&mut self, map: &str, more: bool, body: Vec<u8>) -> Option<Vec<u8>> {
        let mut acc = self.partial.remove(map).unwrap_or_default();
        if acc.is_empty() {
            acc = body;
        } else {
            acc.extend_from_slice(&body);
        }
        if more {
            if acc.len() > MAX_BODY {
                return None;
            }
            self.partial.insert(map.to_owned(), acc);
            return None;
        }
        Some(acc)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::json;

    use super::*;

    fn encode(header: &serde_json::Value, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write_frame(&mut out, header, body).unwrap();
        out
    }

    #[test]
    fn round_trip_with_and_without_body() {
        let mut stream = encode(&json!({"t": "Shutdown"}), b"");
        stream.extend(encode(&json!({"t": "Markers", "map": "w", "more": false}), b"{\"a\":1}"));
        let mut r = Cursor::new(stream);
        let a = read_frame(&mut r).unwrap().unwrap();
        assert_eq!((a.kind(), a.body.len()), ("Shutdown", 0));
        let b = read_frame(&mut r).unwrap().unwrap();
        assert_eq!(b.kind(), "Markers");
        assert_eq!(b.body, b"{\"a\":1}");
        assert!(read_frame(&mut r).unwrap().is_none());
    }

    #[test]
    fn layout_is_big_endian_lengths() {
        let bytes = encode(&json!({"t": "X"}), b"ab");
        let header = br#"{"t":"X"}"#;
        assert_eq!(&bytes[..4], &((4 + header.len() + 2) as u32).to_be_bytes());
        assert_eq!(&bytes[4..8], &(header.len() as u32).to_be_bytes());
        assert_eq!(&bytes[8..8 + header.len()], header);
        assert_eq!(&bytes[8 + header.len()..], b"ab");
    }

    #[test]
    fn truncation_anywhere_is_an_error() {
        let full = encode(&json!({"t": "X"}), b"body");
        for cut in 1..full.len() {
            let err = read_frame(&mut Cursor::new(&full[..cut])).unwrap_err();
            assert!(matches!(err, IpcError::Truncated), "cut at {cut}: {err}");
        }
    }

    #[test]
    fn rejects_bad_lengths_and_headers() {
        let mut short = 3u32.to_be_bytes().to_vec();
        short.extend(0u32.to_be_bytes());
        assert!(matches!(read_frame(&mut Cursor::new(short)), Err(IpcError::BadLength(3))));

        let mut overlong_header = 8u32.to_be_bytes().to_vec();
        overlong_header.extend(9u32.to_be_bytes());
        assert!(matches!(read_frame(&mut Cursor::new(overlong_header)), Err(IpcError::BadLength(8))));

        let mut huge = u32::MAX.to_be_bytes().to_vec();
        huge.extend(2u32.to_be_bytes());
        assert!(matches!(read_frame(&mut Cursor::new(huge)), Err(IpcError::TooLarge { .. })));

        let no_type = encode(&json!({"id": 1}), b"");
        assert!(matches!(read_frame(&mut Cursor::new(no_type)), Err(IpcError::NoType)));

        let mut not_json = 7u32.to_be_bytes().to_vec();
        not_json.extend(3u32.to_be_bytes());
        not_json.extend(b"{x}");
        assert!(matches!(read_frame(&mut Cursor::new(not_json)), Err(IpcError::Header(_))));
    }

    #[test]
    fn oversized_body_is_refused_on_write() {
        let body = vec![0u8; MAX_BODY + 1];
        assert!(matches!(write_frame(&mut Vec::new(), &json!({"t": "X"}), &body), Err(IpcError::TooLarge { .. })));
    }

    /// A reader that hands out one byte per call, like a slow pipe.
    struct Trickle(Cursor<Vec<u8>>);

    impl Read for Trickle {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = buf.len().min(1);
            self.0.read(&mut buf[..n])
        }
    }

    #[test]
    fn short_reads_are_reassembled() {
        let mut r = Trickle(Cursor::new(encode(&json!({"t": "Log", "msg": "hi"}), b"xyz")));
        let f = read_frame(&mut r).unwrap().unwrap();
        assert_eq!((f.kind(), f.body.as_slice()), ("Log", b"xyz".as_slice()));
    }

    #[test]
    fn marker_parts_join_per_map() {
        let mut a = MarkerAssembler::default();
        assert_eq!(a.push("w", true, b"{\"x\":".to_vec()), None);
        assert_eq!(a.push("n", false, b"{}".to_vec()), Some(b"{}".to_vec()));
        assert_eq!(a.push("w", false, b"1}".to_vec()), Some(b"{\"x\":1}".to_vec()));
        assert_eq!(a.push("w", false, b"{}".to_vec()), Some(b"{}".to_vec()));
    }
}
