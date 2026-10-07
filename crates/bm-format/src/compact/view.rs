//! Zero-copy view of a PRBM file in `PRBMWriter`'s layout (see [`crate::prbm`]).

/// Name, type byte and bytes per vertex of the 7 attributes, in file order.
pub(super) const ATTRS: [(&str, u8, usize); 7] = [
    ("position", 0x21, 12),
    ("normal", 0x63, 3),
    ("color", 0x67, 3),
    ("uv", 0x11, 8),
    ("ao", 0x47, 1),
    ("blocklight", 0x03, 1),
    ("sunlight", 0x03, 1),
];

pub(super) const POSITION: usize = 0;
pub(super) const NORMAL: usize = 1;
pub(super) const COLOR: usize = 2;
pub(super) const UV: usize = 3;
pub(super) const AO: usize = 4;
pub(super) const BLOCKLIGHT: usize = 5;
pub(super) const SUNLIGHT: usize = 6;

/// Vertices of quad `q` are PRBM vertices `6q + QV[i]`: triangles (v0,v1,v2),(v0,v2,v3) → 0,1,2,(0),(2),5.
pub(super) const QV: [usize; 4] = [0, 1, 2, 5];

pub(super) struct PrbmView<'a> {
    pub vertices: usize,
    /// Attribute payloads, indexed like [`ATTRS`].
    pub attrs: [&'a [u8]; 7],
    /// `(material, start, count)` triples without the `-1` terminator.
    pub groups: Vec<[i32; 3]>,
}

/// `None` unless `buf` has PRBMWriter's structure (padding bytes are not checked; the encoder's round-trip is).
pub(super) fn parse(buf: &[u8]) -> Option<PrbmView<'_>> {
    if buf.len() < 8 || buf[0] != 1 || buf[1] != 7 || buf[5..8] != [0, 0, 0] {
        return None;
    }
    let vertices = u32::from_le_bytes([buf[2], buf[3], buf[4], 0]) as usize;
    let mut pos = 8;
    let mut attrs: [&[u8]; 7] = [&[]; 7];
    for (slot, (name, ty, size)) in attrs.iter_mut().zip(ATTRS) {
        let head = buf.get(pos..pos + name.len() + 2)?;
        if &head[..name.len()] != name.as_bytes() || head[name.len()] != 0 || head[name.len() + 1] != ty {
            return None;
        }
        pos = (pos + name.len() + 2).next_multiple_of(4);
        *slot = buf.get(pos..pos + vertices * size)?;
        pos += vertices * size;
    }
    pos = pos.next_multiple_of(4);
    let rest = buf.get(pos..)?;
    if rest.len() < 4 || (rest.len() - 4) % 12 != 0 || rest[rest.len() - 4..] != (-1i32).to_le_bytes() {
        return None;
    }
    let ints = |c: &[u8; 12]| -> [i32; 3] {
        let i = |o: usize| i32::from_le_bytes(c[o..o + 4].try_into().unwrap());
        [i(0), i(4), i(8)]
    };
    let groups = rest[..rest.len() - 4].as_chunks::<12>().0.iter().map(ints).collect();
    Some(PrbmView { vertices, attrs, groups })
}

impl PrbmView<'_> {
    /// Every triangle pair is a quad (pos/uv/ao of `6q+3 == 6q`, `6q+4 == 6q+2`) and the groups tile the vertex
    /// range in quad-sized runs, so `(material, quads)` pairs restore them.
    pub fn is_quad_shaped(&self) -> bool {
        if !self.vertices.is_multiple_of(6) {
            return false;
        }
        let mut next = 0i64;
        for &[_, start, count] in &self.groups {
            if i64::from(start) != next || count < 0 || count % 6 != 0 {
                return false;
            }
            next += i64::from(count);
        }
        if next != self.vertices as i64 {
            return false;
        }
        [POSITION, UV, AO].into_iter().all(|a| {
            let size = ATTRS[a].2;
            self.attrs[a].chunks_exact(6 * size).all(|q| {
                let v = |i: usize| &q[i * size..(i + 1) * size];
                v(3) == v(0) && v(4) == v(2)
            })
        })
    }
}
