//! Zero-copy, lazy NBT reader. Compounds and lists are views over the input bytes and are only parsed when
//! iterated, so a reader that wants three fields of a chunk walks the rest without allocating anything.
//! Nesting is capped at Minecraft's 512 so malformed data can't overflow the stack.

mod array;
mod value;
mod writer;

pub use array::BeArray;
pub use value::{Compound, Entries, List, Tag};
pub use writer::Writer;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("NBT truncated")]
    Truncated,
    #[error("unknown NBT tag type {0}")]
    BadTag(u8),
    #[error("NBT nested deeper than {MAX_DEPTH}")]
    TooDeep,
    #[error("NBT root is not a compound")]
    RootNotCompound,
}

pub type Result<T> = std::result::Result<T, Error>;

const MAX_DEPTH: u32 = 512;

pub mod tag {
    pub const END: u8 = 0;
    pub const BYTE: u8 = 1;
    pub const SHORT: u8 = 2;
    pub const INT: u8 = 3;
    pub const LONG: u8 = 4;
    pub const FLOAT: u8 = 5;
    pub const DOUBLE: u8 = 6;
    pub const BYTE_ARRAY: u8 = 7;
    pub const STRING: u8 = 8;
    pub const LIST: u8 = 9;
    pub const COMPOUND: u8 = 10;
    pub const INT_ARRAY: u8 = 11;
    pub const LONG_ARRAY: u8 = 12;
}

/// The root compound of an NBT document (Java edition: a named compound tag).
pub fn read_root(buf: &[u8]) -> Result<Compound<'_>> {
    let (&ty, rest) = buf.split_first().ok_or(Error::Truncated)?;
    if ty != tag::COMPOUND {
        return Err(Error::RootNotCompound);
    }
    let name_len = u16::from_be_bytes(take::<2>(rest, 0)?) as usize;
    let body = rest.get(2 + name_len..).ok_or(Error::Truncated)?;
    let len = value::payload_len(tag::COMPOUND, body, 0)?;
    Ok(Compound::new(&body[..len]))
}

fn take<const N: usize>(buf: &[u8], at: usize) -> Result<[u8; N]> {
    buf.get(at..at + N).and_then(|s| s.try_into().ok()).ok_or(Error::Truncated)
}

#[cfg(test)]
mod tests;
