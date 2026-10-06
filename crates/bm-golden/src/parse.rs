//! PRBM decoder (`PRBMWriter.java` / `PRBMLoader.js`, BlueMap v5.27). Attributes are looked up by name
//! and type-checked, so a reordered or extended format fails loudly instead of misreading.

use std::collections::HashMap;

use anyhow::{Context, Result, bail, ensure};

use crate::reader::Reader;
use crate::tile::{Group, Tile};

enum Values {
    F32(Vec<f32>),
    I8(Vec<i8>),
    U8(Vec<u8>),
}

struct Attribute {
    cardinality: usize,
    values: Values,
}

pub fn parse(buf: &[u8]) -> Result<Tile> {
    let mut r = Reader::new(buf);
    let version = r.u8()?;
    ensure!(version == 1, "unsupported PRBM version {version}");
    let flags = r.u8()?;
    ensure!(flags & 0b1110_0000 == 0, "indexed or big-endian PRBM not supported (flags {flags:#010b})");
    let attr_count = flags & 0x1f;
    let vertices = r.u24()? as usize;
    ensure!(vertices.is_multiple_of(3), "vertex count {vertices} not a multiple of 3");
    r.u24()?;

    let mut attrs = HashMap::new();
    for _ in 0..attr_count {
        let name = r.cstr()?.to_owned();
        let attr = read_attribute(&mut r, vertices).with_context(|| format!("attribute {name}"))?;
        attrs.insert(name, attr);
    }
    let groups = read_groups(&mut r, vertices)?;

    let mut take = |name: &str| attrs.remove(name).with_context(|| format!("missing attribute {name}"));
    Ok(Tile {
        position: take("position")?.f32s::<3>()?,
        normal: take("normal")?.i8s::<3>()?,
        color: take("color")?.u8s::<3>()?,
        uv: take("uv")?.f32s::<2>()?,
        ao: flat(take("ao")?.u8s::<1>()?),
        blocklight: flat(take("blocklight")?.i8s::<1>()?),
        sunlight: flat(take("sunlight")?.i8s::<1>()?),
        groups,
    })
}

fn read_attribute(r: &mut Reader, vertices: usize) -> Result<Attribute> {
    let flags = r.u8()?;
    ensure!(flags & 0x80 == 0, "integer-typed attributes not supported");
    let cardinality = ((flags >> 4) & 0x3) as usize + 1;
    let n = vertices * cardinality;
    r.align4();
    let values = match flags & 0x0f {
        1 => Values::F32(r.bytes(n * 4)?.as_chunks::<4>().0.iter().map(|&b| f32::from_le_bytes(b)).collect()),
        3 => Values::I8(r.bytes(n)?.iter().map(|&b| b as i8).collect()),
        7 => Values::U8(r.bytes(n)?.to_vec()),
        e => bail!("unsupported encoding {e}"),
    };
    Ok(Attribute { cardinality, values })
}

fn read_groups(r: &mut Reader, vertices: usize) -> Result<Vec<Group>> {
    r.align4();
    let mut groups = Vec::new();
    let mut expected_start = 0;
    loop {
        let material = r.i32()?;
        if material == -1 {
            break;
        }
        let (start, count) = (r.i32()?, r.i32()?);
        ensure!(material >= 0 && start == expected_start && count > 0, "bad group {material}/{start}/{count}");
        expected_start = start.checked_add(count).context("group table overflows i32")?;
        groups.push(Group { material: material as u32, start: start as u32, count: count as u32 });
    }
    ensure!(expected_start as usize == vertices, "groups cover {expected_start} of {vertices} vertices");
    ensure!(r.at_end(), "trailing bytes after group table");
    Ok(groups)
}

impl Attribute {
    fn check<const N: usize>(&self) -> Result<()> {
        ensure!(self.cardinality == N, "cardinality {} != {N}", self.cardinality);
        Ok(())
    }

    fn f32s<const N: usize>(self) -> Result<Vec<[f32; N]>> {
        self.check::<N>()?;
        let Values::F32(v) = self.values else { bail!("expected f32 values") };
        Ok(chunk(&v))
    }

    fn i8s<const N: usize>(self) -> Result<Vec<[i8; N]>> {
        self.check::<N>()?;
        let Values::I8(v) = self.values else { bail!("expected i8 values") };
        Ok(chunk(&v))
    }

    fn u8s<const N: usize>(self) -> Result<Vec<[u8; N]>> {
        self.check::<N>()?;
        let Values::U8(v) = self.values else { bail!("expected u8 values") };
        Ok(chunk(&v))
    }
}

fn chunk<T: Copy, const N: usize>(v: &[T]) -> Vec<[T; N]> {
    v.as_chunks::<N>().0.to_vec()
}

fn flat<T: Copy>(v: Vec<[T; 1]>) -> Vec<T> {
    v.into_iter().map(|[x]| x).collect()
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
