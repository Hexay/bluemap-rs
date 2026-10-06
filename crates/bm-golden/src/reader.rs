//! Little-endian cursor over a PRBM buffer. Alignment is measured from the start of the buffer.

use anyhow::{Result, bail};

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len());
        let Some(end) = end else {
            bail!("unexpected end of PRBM at {} (+{n}, len {})", self.pos, self.buf.len());
        };
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    pub fn u24(&mut self) -> Result<u32> {
        let b = self.bytes(3)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], 0]))
    }

    pub fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into()?))
    }

    pub fn cstr(&mut self) -> Result<&'a str> {
        let Some(rest) = self.buf.get(self.pos..) else { bail!("unexpected end of PRBM at {}", self.pos) };
        let Some(len) = rest.iter().position(|&b| b == 0) else { bail!("unterminated string") };
        let s = std::str::from_utf8(&rest[..len])?;
        self.pos += len + 1;
        Ok(s)
    }

    pub fn align4(&mut self) {
        self.pos = self.pos.next_multiple_of(4);
    }

    pub fn at_end(&self) -> bool {
        self.pos >= self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn little_endian_scalars() {
        let buf = [0xab, 0x01, 0x02, 0x03, 0xfe, 0xff, 0xff, 0xff];
        let mut r = Reader::new(&buf);
        assert_eq!(r.u8().unwrap(), 0xab);
        assert_eq!(r.u24().unwrap(), 0x030201);
        assert_eq!(r.i32().unwrap(), -2);
        assert!(r.at_end());
    }

    #[test]
    fn cstr_consumes_terminator() {
        let mut r = Reader::new(b"uv\0ao\0x");
        assert_eq!(r.cstr().unwrap(), "uv");
        assert_eq!(r.cstr().unwrap(), "ao");
        assert_eq!(r.u8().unwrap(), b'x');
        assert!(r.at_end());
    }

    #[test]
    fn align4_is_relative_to_buffer_start() {
        let buf = [0u8; 12];
        let mut r = Reader::new(&buf);
        r.align4();
        assert_eq!(r.bytes(0).unwrap().len(), 0);
        r.u8().unwrap();
        r.align4();
        assert_eq!(r.pos, 4);
        r.align4();
        assert_eq!(r.pos, 4);
        r.bytes(5).unwrap();
        r.align4();
        assert_eq!(r.pos, 12);
    }

    #[test]
    fn short_reads_error() {
        assert!(Reader::new(&[1, 2]).u24().is_err());
        assert!(Reader::new(&[1, 2, 3]).i32().is_err());
        assert!(Reader::new(&[]).u8().is_err());
        let mut r = Reader::new(&[1]);
        r.u8().unwrap();
        assert!(r.bytes(usize::MAX).is_err(), "length overflow must not wrap");
    }

    #[test]
    fn failed_read_does_not_advance() {
        let mut r = Reader::new(&[7, 8]);
        assert!(r.i32().is_err());
        assert_eq!(r.u8().unwrap(), 7);
    }

    #[test]
    fn bad_strings_error() {
        assert!(Reader::new(b"no terminator").cstr().is_err());
        assert!(Reader::new(&[0xff, 0xfe, 0]).cstr().is_err());
    }

    #[test]
    fn read_after_align_past_end_errors() {
        let mut r = Reader::new(&[1, 2, 3, 4, 5]);
        r.u8().unwrap();
        r.bytes(4).unwrap();
        r.align4();
        assert!(r.at_end());
        assert!(r.bytes(0).is_err());
    }

    #[test]
    fn cstr_after_align_past_end_errors() {
        let mut r = Reader::new(&[1]);
        r.u8().unwrap();
        r.align4();
        assert!(r.cstr().is_err());
    }
}
