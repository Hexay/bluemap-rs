//! Length-prefixed streams, a bounds-checked reader and 4-byte-plane integer arrays.

use super::CompactError;

/// Appends `[u32 len][bytes written by f]`.
pub(super) fn stream(body: &mut Vec<u8>, f: impl FnOnce(&mut Vec<u8>)) {
    let at = body.len();
    body.extend([0; 4]);
    f(body);
    let len = (body.len() - at - 4) as u32;
    body[at..at + 4].copy_from_slice(&len.to_le_bytes());
}

/// `u32` values as 4 planes (all low bytes, …, all high bytes): sparse small values leave the high planes zero.
pub(super) fn put_planes(out: &mut Vec<u8>, values: impl Iterator<Item = u32> + Clone) {
    for shift in [0, 8, 16, 24] {
        out.extend(values.clone().map(|v| (v >> shift) as u8));
    }
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

    /// `n` values stored by [`put_planes`].
    pub fn planes(&mut self, n: usize) -> Result<impl Iterator<Item = u32> + 'a, CompactError> {
        let p = self.take(n.checked_mul(4).ok_or(CompactError::Corrupt("count"))?)?;
        Ok((0..n).map(move |i| u32::from_le_bytes([p[i], p[n + i], p[2 * n + i], p[3 * n + i]])))
    }

    pub fn is_empty(&self) -> bool {
        self.pos == self.buf.len()
    }
}
