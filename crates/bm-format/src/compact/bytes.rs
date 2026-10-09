//! Length-prefixed streams and a bounds-checked reader.

use super::CompactError;

/// Appends `[u32 len][bytes written by f]`.
pub(super) fn stream(body: &mut Vec<u8>, f: impl FnOnce(&mut Vec<u8>)) {
    let at = body.len();
    body.extend([0; 4]);
    f(body);
    let len = (body.len() - at - 4) as u32;
    body[at..at + 4].copy_from_slice(&len.to_le_bytes());
}

pub(super) struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], CompactError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len()).ok_or(CompactError::Corrupt("truncated"))?;
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    /// Everything not yet read.
    pub fn rest(&mut self) -> &'a [u8] {
        let out = &self.buf[self.pos..];
        self.pos = self.buf.len();
        out
    }

    pub fn u8(&mut self) -> Result<u8, CompactError> {
        Ok(self.take(1)?[0])
    }

    pub fn u32(&mut self) -> Result<u32, CompactError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    /// One `[u32 len][bytes]` stream, which must be exactly `expect` bytes long when given.
    pub fn stream(&mut self, expect: Option<usize>) -> Result<Reader<'a>, CompactError> {
        let len = self.u32()? as usize;
        if expect.is_some_and(|e| e != len) {
            return Err(CompactError::Corrupt("stream length"));
        }
        Ok(Reader::new(self.take(len)?))
    }

    pub fn is_empty(&self) -> bool {
        self.pos == self.buf.len()
    }
}
