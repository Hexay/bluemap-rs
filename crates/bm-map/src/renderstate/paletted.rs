//! `PalettedArrayAdapter` + `RegistryAdapter` (`core/util/nbt/*`): `{palette: [string…], data: byte[]}` where the
//! palette is ordered by first occurrence in the array.

use bm_nbt::{Tag, Writer};

use super::{Error, Result};

const FIELD: &str = "tile-states";

/// Missing `data` yields an empty array (the caller's length check then resets it), like Java.
pub fn read<T: Copy>(value: Tag<'_>, parse: impl Fn(&str) -> T) -> Result<Vec<T>> {
    let compound = value.as_compound().ok_or(Error::WrongType(FIELD))?;
    let mut palette = None;
    let mut data = None;
    for (name, tag) in compound.entries() {
        match name {
            b"palette" => palette = Some(read_palette(tag, &parse)?),
            b"data" => data = Some(tag.as_byte_array().ok_or(Error::WrongType("data"))?),
            _ => {}
        }
    }
    let palette = palette.filter(|p| !p.is_empty()).ok_or(Error::EmptyPalette)?;
    data.unwrap_or_default()
        .iter()
        // Java indexes with a signed byte: entries >= 128 are out of range
        .map(|&i| palette.get(i as i8 as usize).copied().ok_or(Error::PaletteIndex { size: palette.len(), index: i }))
        .collect()
}

fn read_palette<T>(value: Tag<'_>, parse: impl Fn(&str) -> T) -> Result<Vec<T>> {
    let list = value.as_list().ok_or(Error::WrongType("palette"))?;
    list.iter()
        .map(|t| match t {
            Tag::String(b) => Ok(parse(&String::from_utf8_lossy(b))),
            _ => Err(Error::WrongType("palette")),
        })
        .collect()
}

pub fn write<T: Copy + Eq>(w: &mut Writer, name: &str, values: &[T], key: impl Fn(T) -> &'static str) {
    let mut palette: Vec<T> = Vec::new();
    let data: Vec<u8> = values
        .iter()
        .map(|v| match palette.iter().position(|p| p == v) {
            Some(i) => i as u8,
            None => {
                palette.push(*v);
                (palette.len() - 1) as u8
            }
        })
        .collect();
    let keys: Vec<&str> = palette.into_iter().map(key).collect();
    w.begin_compound(name).string_list("palette", &keys).byte_array("data", &data).end_compound();
}
