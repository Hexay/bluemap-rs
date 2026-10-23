//! A parsed document whose compound and list lengths were recorded by its validating walk.
//!
//! Views only know where a payload starts, so stepping past a nested compound or list means walking all of it
//! again, once per level the reader descends: a palette entry of a chunk section sat under 7 such walks, half of
//! a chunk parse. [`Document`] keeps every container's length from the one walk [`Document::parse`] makes, and
//! views made from it look lengths up instead.

use crate::value::{Compound, Record, payload_len_with};
use crate::{Error, Result, tag, take};

/// Payload lengths by start offset, in document order (so sorted by offset).
#[derive(Debug)]
pub(crate) struct Index {
    base: usize,
    spans: Vec<(u32, u32)>,
}

impl Index {
    /// The recorded length of the compound or list payload starting at `at`, which must lie in the document.
    pub(crate) fn len_at(&self, at: &[u8]) -> Option<usize> {
        let offset = u32::try_from(at.as_ptr() as usize - self.base).ok()?;
        let i = self.spans.binary_search_by_key(&offset, |s| s.0).ok()?;
        Some(self.spans[i].1 as usize)
    }
}

impl Record for Index {
    fn open(&mut self, at: &[u8]) -> usize {
        // offsets past u32 are left out; their lookups fall back to walking
        let offset = u32::try_from(at.as_ptr() as usize - self.base).unwrap_or(u32::MAX);
        self.spans.push((offset, 0));
        self.spans.len() - 1
    }

    fn close(&mut self, slot: usize, len: usize) {
        self.spans[slot].1 = u32::try_from(len).unwrap_or(0);
        if self.spans[slot].1 as usize != len {
            self.spans[slot].0 = u32::MAX;
        }
    }
}

/// An NBT document read like [`crate::read_root`], with an [`Index`] its views use.
pub struct Document<'a> {
    body: &'a [u8],
    index: Index,
}

impl<'a> Document<'a> {
    pub fn parse(buf: &'a [u8]) -> Result<Self> {
        let (&ty, rest) = buf.split_first().ok_or(Error::Truncated)?;
        if ty != tag::COMPOUND {
            return Err(Error::RootNotCompound);
        }
        let name_len = u16::from_be_bytes(take::<2>(rest, 0)?) as usize;
        let body = rest.get(2 + name_len..).ok_or(Error::Truncated)?;
        let mut index = Index { base: body.as_ptr() as usize, spans: Vec::with_capacity(body.len() / 32) };
        let len = payload_len_with(tag::COMPOUND, body, 0, &mut index)?;
        // a u32::MAX offset marks spans that cannot be looked up; it also keeps the order sorted
        index.spans.retain(|s| s.0 != u32::MAX);
        Ok(Self { body: &body[..len], index })
    }

    pub fn root(&self) -> Compound<'_> {
        Compound::indexed(self.body, &self.index)
    }
}
