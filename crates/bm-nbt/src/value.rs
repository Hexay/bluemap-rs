use crate::{BeArray, Error, MAX_DEPTH, Result, tag, take};

#[derive(Clone, Copy, Debug)]
pub enum Tag<'a> {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(&'a [u8]),
    /// Modified UTF-8 bytes.
    String(&'a [u8]),
    List(List<'a>),
    Compound(Compound<'a>),
    IntArray(BeArray<'a, 4>),
    LongArray(BeArray<'a, 8>),
}

impl<'a> Tag<'a> {
    pub fn as_compound(&self) -> Option<Compound<'a>> {
        if let Self::Compound(c) = self { Some(*c) } else { None }
    }

    pub fn as_list(&self) -> Option<List<'a>> {
        if let Self::List(l) = self { Some(*l) } else { None }
    }

    /// The string if it is valid UTF-8, which identifiers always are (MUTF-8 differs only for NUL and astral chars).
    pub fn as_str(&self) -> Option<&'a str> {
        if let Self::String(s) = self { std::str::from_utf8(s).ok() } else { None }
    }

    /// Any integer tag, widened: Minecraft has changed the width of several fields between versions.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Self::Byte(v) => Some(v.into()),
            Self::Short(v) => Some(v.into()),
            Self::Int(v) => Some(v.into()),
            Self::Long(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Self::Float(v) => Some(v.into()),
            Self::Double(v) => Some(v),
            _ => self.as_i64().map(|v| v as f64),
        }
    }

    pub fn as_byte_array(&self) -> Option<&'a [u8]> {
        if let Self::ByteArray(b) = self { Some(b) } else { None }
    }

    pub fn as_int_array(&self) -> Option<BeArray<'a, 4>> {
        if let Self::IntArray(a) = self { Some(*a) } else { None }
    }

    pub fn as_long_array(&self) -> Option<BeArray<'a, 8>> {
        if let Self::LongArray(a) = self { Some(*a) } else { None }
    }
}

/// Entries up to and including the END tag. Always validated by [`payload_len`] before construction, so walking
/// it again cannot fail.
#[derive(Clone, Copy, Debug)]
pub struct Compound<'a> {
    body: &'a [u8],
}

impl<'a> Compound<'a> {
    pub(crate) fn new(body: &'a [u8]) -> Self {
        Self { body }
    }

    pub fn entries(&self) -> Entries<'a> {
        Entries { rest: self.body }
    }

    /// Scans for `name`; use [`Compound::entries`] to read several fields in one pass.
    pub fn get(&self, name: &str) -> Option<Tag<'a>> {
        self.entries().find(|(n, _)| *n == name.as_bytes()).map(|(_, t)| t)
    }

    pub fn compound(&self, name: &str) -> Option<Compound<'a>> {
        self.get(name)?.as_compound()
    }

    pub fn list(&self, name: &str) -> Option<List<'a>> {
        self.get(name)?.as_list()
    }

    pub fn str(&self, name: &str) -> Option<&'a str> {
        self.get(name)?.as_str()
    }

    pub fn i64(&self, name: &str) -> Option<i64> {
        self.get(name)?.as_i64()
    }
}

pub struct Entries<'a> {
    rest: &'a [u8],
}

impl<'a> Iterator for Entries<'a> {
    type Item = (&'a [u8], Tag<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        let (&ty, rest) = self.rest.split_first()?;
        if ty == tag::END {
            self.rest = &[];
            return None;
        }
        let name_len = u16::from_be_bytes(take::<2>(rest, 0).ok()?) as usize;
        let name = rest.get(2..2 + name_len)?;
        let (value, len) = parse(ty, &rest[2 + name_len..]).ok()?;
        self.rest = &rest[2 + name_len + len..];
        Some((name, value))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct List<'a> {
    elem: u8,
    len: usize,
    body: &'a [u8],
}

impl<'a> List<'a> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn elem_type(&self) -> u8 {
        self.elem
    }

    pub fn iter(self) -> impl Iterator<Item = Tag<'a>> + 'a {
        let (elem, mut rest) = (self.elem, self.body);
        (0..self.len).map_while(move |_| {
            let (value, len) = parse(elem, rest).ok()?;
            rest = &rest[len..];
            Some(value)
        })
    }

    /// The compound elements; empty if the list holds anything else.
    pub fn compounds(self) -> impl Iterator<Item = Compound<'a>> + 'a {
        let list = if self.elem == tag::COMPOUND { self } else { Self { elem: tag::END, len: 0, body: &[] } };
        list.iter().filter_map(|t| t.as_compound())
    }
}

fn parse(ty: u8, buf: &[u8]) -> Result<(Tag<'_>, usize)> {
    let len = payload_len(ty, buf, 0)?;
    let b = &buf[..len];
    let tag = match ty {
        tag::BYTE => Tag::Byte(b[0] as i8),
        tag::SHORT => Tag::Short(i16::from_be_bytes(take(b, 0)?)),
        tag::INT => Tag::Int(i32::from_be_bytes(take(b, 0)?)),
        tag::LONG => Tag::Long(i64::from_be_bytes(take(b, 0)?)),
        tag::FLOAT => Tag::Float(f32::from_be_bytes(take(b, 0)?)),
        tag::DOUBLE => Tag::Double(f64::from_be_bytes(take(b, 0)?)),
        tag::BYTE_ARRAY => Tag::ByteArray(&b[4..]),
        tag::STRING => Tag::String(&b[2..]),
        tag::LIST => {
            let n = i32::from_be_bytes(take(b, 1)?).max(0) as usize;
            Tag::List(List { elem: b[0], len: if b[0] == tag::END { 0 } else { n }, body: &b[5..] })
        }
        tag::COMPOUND => Tag::Compound(Compound::new(b)),
        tag::INT_ARRAY => Tag::IntArray(BeArray::new(&b[4..])),
        tag::LONG_ARRAY => Tag::LongArray(BeArray::new(&b[4..])),
        _ => return Err(Error::BadTag(ty)),
    };
    Ok((tag, len))
}

/// Bytes taken by a payload of type `ty` at the start of `buf`, validating everything nested in it.
pub(crate) fn payload_len(ty: u8, buf: &[u8], depth: u32) -> Result<usize> {
    if depth > MAX_DEPTH {
        return Err(Error::TooDeep);
    }
    let count = |at: usize| -> Result<usize> {
        let n = i32::from_be_bytes(take(buf, at)?);
        usize::try_from(n).map_err(|_| Error::Truncated)
    };
    let len = match ty {
        tag::BYTE => 1,
        tag::SHORT => 2,
        tag::INT | tag::FLOAT => 4,
        tag::LONG | tag::DOUBLE => 8,
        tag::BYTE_ARRAY => 4 + count(0)?,
        tag::STRING => 2 + u16::from_be_bytes(take(buf, 0)?) as usize,
        tag::INT_ARRAY => 4 + count(0)?.checked_mul(4).ok_or(Error::Truncated)?,
        tag::LONG_ARRAY => 4 + count(0)?.checked_mul(8).ok_or(Error::Truncated)?,
        tag::LIST => {
            let elem = *buf.first().ok_or(Error::Truncated)?;
            let n = if elem == tag::END { 0 } else { count(1)? };
            let mut pos = 5;
            for _ in 0..n {
                pos += payload_len(elem, buf.get(pos..).ok_or(Error::Truncated)?, depth + 1)?;
            }
            pos
        }
        tag::COMPOUND => {
            let mut pos = 0;
            loop {
                let ty = *buf.get(pos).ok_or(Error::Truncated)?;
                pos += 1;
                if ty == tag::END {
                    break pos;
                }
                pos += 2 + u16::from_be_bytes(take(buf, pos)?) as usize;
                pos += payload_len(ty, buf.get(pos..).ok_or(Error::Truncated)?, depth + 1)?;
            }
        }
        _ => return Err(Error::BadTag(ty)),
    };
    if len > buf.len() { Err(Error::Truncated) } else { Ok(len) }
}
