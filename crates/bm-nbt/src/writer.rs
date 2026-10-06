use crate::tag;

/// Builds an NBT document. Named tags go into the innermost open compound; every `begin_compound` and every element
/// started by `begin_compound_list` must be closed with `end_compound`.
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    /// Opens the root compound (named `""`, as Minecraft writes it).
    pub fn new() -> Self {
        let mut w = Self { buf: Vec::new() };
        w.header(tag::COMPOUND, "");
        w
    }

    /// Closes the root compound and returns the document.
    pub fn finish(mut self) -> Vec<u8> {
        self.buf.push(tag::END);
        self.buf
    }

    fn header(&mut self, ty: u8, name: &str) {
        self.buf.push(ty);
        self.raw_str(name);
    }

    fn raw_str(&mut self, s: &str) {
        let bytes = &s.as_bytes()[..s.len().min(u16::MAX as usize)];
        self.buf.extend((bytes.len() as u16).to_be_bytes());
        self.buf.extend(bytes);
    }

    fn len(&mut self, n: usize) {
        self.buf.extend((n as i32).to_be_bytes());
    }

    pub fn byte(&mut self, name: &str, v: i8) -> &mut Self {
        self.header(tag::BYTE, name);
        self.buf.push(v as u8);
        self
    }

    pub fn short(&mut self, name: &str, v: i16) -> &mut Self {
        self.header(tag::SHORT, name);
        self.buf.extend(v.to_be_bytes());
        self
    }

    pub fn int(&mut self, name: &str, v: i32) -> &mut Self {
        self.header(tag::INT, name);
        self.buf.extend(v.to_be_bytes());
        self
    }

    pub fn long(&mut self, name: &str, v: i64) -> &mut Self {
        self.header(tag::LONG, name);
        self.buf.extend(v.to_be_bytes());
        self
    }

    pub fn float(&mut self, name: &str, v: f32) -> &mut Self {
        self.header(tag::FLOAT, name);
        self.buf.extend(v.to_be_bytes());
        self
    }

    pub fn double(&mut self, name: &str, v: f64) -> &mut Self {
        self.header(tag::DOUBLE, name);
        self.buf.extend(v.to_be_bytes());
        self
    }

    pub fn string(&mut self, name: &str, v: &str) -> &mut Self {
        self.header(tag::STRING, name);
        self.raw_str(v);
        self
    }

    pub fn byte_array(&mut self, name: &str, v: &[u8]) -> &mut Self {
        self.header(tag::BYTE_ARRAY, name);
        self.len(v.len());
        self.buf.extend(v);
        self
    }

    pub fn int_array(&mut self, name: &str, v: &[i32]) -> &mut Self {
        self.header(tag::INT_ARRAY, name);
        self.len(v.len());
        v.iter().for_each(|x| self.buf.extend(x.to_be_bytes()));
        self
    }

    pub fn long_array(&mut self, name: &str, v: &[u64]) -> &mut Self {
        self.header(tag::LONG_ARRAY, name);
        self.len(v.len());
        v.iter().for_each(|x| self.buf.extend(x.to_be_bytes()));
        self
    }

    pub fn string_list(&mut self, name: &str, v: &[&str]) -> &mut Self {
        self.header(tag::LIST, name);
        self.buf.push(tag::STRING);
        self.len(v.len());
        v.iter().for_each(|s| self.raw_str(s));
        self
    }

    pub fn begin_compound(&mut self, name: &str) -> &mut Self {
        self.header(tag::COMPOUND, name);
        self
    }

    /// Starts a list of `len` compounds; write each element's entries, then `end_compound`, `len` times.
    pub fn begin_compound_list(&mut self, name: &str, len: usize) -> &mut Self {
        self.header(tag::LIST, name);
        self.buf.push(if len == 0 { tag::END } else { tag::COMPOUND });
        self.len(len);
        self
    }

    pub fn end_compound(&mut self) -> &mut Self {
        self.buf.push(tag::END);
        self
    }
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}
