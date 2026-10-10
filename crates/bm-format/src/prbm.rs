//! Hires tile meshes and their PRBM encoding (`PRBMWriter.java`, read by the webapp's `PRBMLoader.js`):
//! non-indexed little-endian PRWM with 7 attributes, then material groups. Output is byte-identical to BlueMap's.

/// Triangles in BlueMap's `ArrayTileModel` layout: struct of arrays, one entry per triangle. Positions, uvs and ao
/// are per vertex; colour and light per triangle. Triangles must be sorted by material (stable) before writing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileModel {
    /// 9 per triangle: x/z tile-local, y world height.
    pub position: Vec<f32>,
    /// 6 per triangle.
    pub uv: Vec<f32>,
    /// 3 per triangle, 0..1.
    pub ao: Vec<f32>,
    /// 3 per triangle (rgb tint, 0..1).
    pub color: Vec<f32>,
    pub sunlight: Vec<u8>,
    pub blocklight: Vec<u8>,
    /// Texture-gallery id per triangle.
    pub material: Vec<u32>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("tile has {0} vertices; PRBM stores at most 16777215")]
pub struct TooManyVertices(pub usize);

const MAX_VERTICES: usize = 0xFF_FFFF;

impl TileModel {
    pub fn len(&self) -> usize {
        self.material.len()
    }

    pub fn is_empty(&self) -> bool {
        self.material.is_empty()
    }

    pub fn clear(&mut self) {
        self.position.clear();
        self.uv.clear();
        self.ao.clear();
        self.color.clear();
        self.sunlight.clear();
        self.blocklight.clear();
        self.material.clear();
    }

    /// Encodes the model into `out` (replacing its contents).
    pub fn write_prbm(&self, out: &mut Vec<u8>) -> Result<(), TooManyVertices> {
        let triangles = self.len();
        let vertices = triangles * 3;
        if vertices > MAX_VERTICES {
            return Err(TooManyVertices(vertices));
        }
        out.clear();
        out.reserve(64 + vertices * 33 + 12 * triangles.min(1024));
        out.extend([1, 0b0000_0111]);
        u24(out, vertices);
        u24(out, 0);

        attribute(out, "position", 0x21);
        floats(out, &self.position);

        attribute(out, "normal", 0x63);
        per_triangle(out, &self.position, |p: &[f32; 9]| surface_normal(p).map(normal_byte));

        attribute(out, "color", 0x67);
        per_triangle(out, &self.color, |c: &[f32; 3]| c.map(unit_byte));

        attribute(out, "uv", 0x11);
        floats(out, &self.uv);

        attribute(out, "ao", 0x47);
        out.extend(self.ao.iter().map(|&v| unit_byte(v)));

        attribute(out, "blocklight", 0x03);
        light(out, &self.blocklight);
        attribute(out, "sunlight", 0x03);
        light(out, &self.sunlight);

        pad(out);
        let mut start = 0;
        for (i, w) in self.material.windows(2).enumerate() {
            if w[0] != w[1] {
                group(out, w[0], start, i + 1);
                start = i + 1;
            }
        }
        if let Some(&last) = self.material.last() {
            group(out, last, start, triangles);
        }
        out.extend((-1i32).to_le_bytes());
        Ok(())
    }
}

fn group(out: &mut Vec<u8>, material: u32, start: usize, end: usize) {
    for v in [material as i32, (start * 3) as i32, ((end - start) * 3) as i32] {
        out.extend(v.to_le_bytes());
    }
}

/// Name, NUL, attribute type byte, then zero padding to 4 bytes from the start of the file.
pub(crate) fn attribute(out: &mut Vec<u8>, name: &str, ty: u8) {
    out.extend(name.as_bytes());
    out.extend([0, ty]);
    pad(out);
}

pub(crate) fn pad(out: &mut Vec<u8>) {
    out.resize(out.len().next_multiple_of(4), 0);
}

pub(crate) fn u24(out: &mut Vec<u8>, v: usize) {
    out.extend(&(v as u32).to_le_bytes()[..3]);
}

/// `count` more bytes at the end of `out`, to be written in place.
fn append(out: &mut Vec<u8>, count: usize) -> &mut [u8] {
    let start = out.len();
    out.resize(start + count, 0);
    &mut out[start..]
}

/// `Float.floatToIntBits` of each value: every NaN collapses to the canonical one.
fn floats(out: &mut Vec<u8>, values: &[f32]) {
    for (b, &v) in append(out, values.len() * 4).as_chunks_mut::<4>().0.iter_mut().zip(values) {
        *b = if v.is_nan() { 0x7fc0_0000u32 } else { v.to_bits() }.to_le_bytes();
    }
}

/// Each triangle's light byte for each of its 3 vertices.
fn light(out: &mut Vec<u8>, values: &[u8]) {
    for (b, &l) in append(out, values.len() * 3).as_chunks_mut::<3>().0.iter_mut().zip(values) {
        *b = [l; 3];
    }
}

/// One 3-byte value from each triangle's `N` inputs, written for each of its 3 vertices.
fn per_triangle<const N: usize, T>(out: &mut Vec<u8>, values: &[T], f: impl Fn(&[T; N]) -> [u8; 3]) {
    let values = values.as_chunks::<N>().0;
    for (b, v) in append(out, values.len() * 9).as_chunks_mut::<9>().0.iter_mut().zip(values) {
        let [x, y, z] = f(v);
        *b = [x, y, z, x, y, z, x, y, z];
    }
}

/// `(int)(v * 255) & 0xFF`: float multiply, saturating cast, low byte.
fn unit_byte(v: f32) -> u8 {
    (v * 255.0) as i32 as u8
}

/// `(byte)(v * 0x80 - 0.5)`: the subtraction happens in double.
pub(crate) fn normal_byte(v: f32) -> u8 {
    ((v * 128.0) as f64 - 0.5) as i32 as u8
}

/// Unit normal of the triangle's plane, in Java's float/double mix.
pub(crate) fn surface_normal(p: &[f32; 9]) -> [f32; 3] {
    let [nx, ny, nz] = plane(p);
    let length = ((nx * nx + ny * ny + nz * nz) as f64).sqrt() as f32;
    [nx / length, ny / length, nz / length]
}

#[inline(always)]
fn plane(p: &[f32; 9]) -> [f32; 3] {
    let (ax, ay, az) = (p[3] - p[0], p[4] - p[1], p[5] - p[2]);
    let (bx, by, bz) = (p[6] - p[0], p[7] - p[1], p[8] - p[2]);
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

/// Magnitudes whose square neither overflows nor leaves f32's normal range.
const SQUARABLE: std::ops::Range<f32> = 1e-12..1e12;

/// `surface_normal(p).map(normal_byte)` without the square root for axis-aligned triangles: there the quotient is
/// within a few ulp of ±1, which [`normal_byte`] maps to one byte per sign. Which axis a face looks along is
/// unpredictable, so the test and the bytes are computed without branches.
// always inlined: as a call, loading the 9 floats the caller just stored one by one stalls on store forwarding
#[inline(always)]
pub(crate) fn normal_bytes(p: &[f32; 9]) -> [u8; 3] {
    let n = plane(p);
    let zero = n.map(|c| u8::from(c == 0.0));
    let squarable = n.map(|c| u8::from(SQUARABLE.contains(&c.abs())));
    let exact = (zero[0] | squarable[0]) & (zero[1] | squarable[1]) & (zero[2] | squarable[2]);
    if zero[0] + zero[1] + zero[2] == 2 && exact == 1 {
        return n.map(|c| u8::from(c > 0.0) * 127 + u8::from(c < 0.0) * 128);
    }
    surface_normal(p).map(normal_byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(material: u32, y: f32) -> TileModel {
        let mut m = TileModel::default();
        for (tri, mat) in [([0., y, 0., 0., y, 1., 1., y, 1.], material), ([0., y, 0., 1., y, 1., 1., y, 0.], material)]
        {
            m.position.extend(tri);
            m.uv.extend([0., 0., 0., 1., 1., 1.]);
            m.ao.extend([1., 0.75, 0.5]);
            m.color.extend([1., 0.5, 0.]);
            m.sunlight.push(15);
            m.blocklight.push(3);
            m.material.push(mat);
        }
        m
    }

    #[test]
    fn header_attributes_and_groups() {
        let mut m = quad(4, 64.);
        let other = quad(9, 65.);
        for (a, b) in [
            (&mut m.position, &other.position),
            (&mut m.uv, &other.uv),
            (&mut m.ao, &other.ao),
            (&mut m.color, &other.color),
        ] {
            a.extend(b);
        }
        m.sunlight.extend(&other.sunlight);
        m.blocklight.extend(&other.blocklight);
        m.material.extend(&other.material);
        let mut out = Vec::new();
        m.write_prbm(&mut out).unwrap();
        assert_eq!(&out[..8], [1, 7, 12, 0, 0, 0, 0, 0]);
        assert_eq!(&out[8..20], b"position\0\x21\0\0");
        assert_eq!(out.len() % 4, 0);
        let groups: Vec<i32> =
            out[out.len() - 28..].chunks(4).map(|b| i32::from_le_bytes(b.try_into().unwrap())).collect();
        assert_eq!(groups, [4, 0, 6, 9, 6, 6, -1]);
    }

    #[test]
    fn java_casts() {
        assert_eq!(unit_byte(1.0), 255);
        assert_eq!(unit_byte(0.5), 127);
        assert_eq!(unit_byte(2.0), 254, "(int)510 & 0xFF");
        assert_eq!(unit_byte(f32::NAN), 0);
        assert_eq!(normal_byte(1.0) as i8, 127);
        assert_eq!(normal_byte(-1.0) as i8, -128);
        assert_eq!(normal_byte(0.0) as i8, 0, "(int)-0.5 truncates to 0");
        assert_eq!(surface_normal(&[0., 0., 0., 0., 0., 1., 1., 0., 0.]), [0., 1., 0.]);
    }

    #[test]
    fn normal_bytes_match_the_square_root_path() {
        let slow = |p: &[f32; 9]| surface_normal(p).map(normal_byte);
        for bits in (0..=u32::MAX).step_by(40_009) {
            let s = f32::from_bits(bits);
            for t in [1.0, -1.0, 3.3e-7, -7.7e5, 0.0, 1e-20, 1e20] {
                for axis in 0..3 {
                    let mut p = [0f32; 9];
                    (p[3 + (axis + 1) % 3], p[6 + (axis + 2) % 3]) = (s, t);
                    assert_eq!(normal_bytes(&p), slow(&p), "{s:e} {t:e} axis {axis}");
                }
            }
        }
        let tilted = [0.25, 1., 0.5, 1.5, 2., 0.75, -1., 0.125, 3.];
        assert_eq!(normal_bytes(&tilted), slow(&tilted));
    }

    #[test]
    fn empty_and_oversized_tiles() {
        let mut out = Vec::new();
        TileModel::default().write_prbm(&mut out).unwrap();
        assert_eq!(&out[out.len() - 4..], (-1i32).to_le_bytes());
        let big = TileModel { material: vec![0; MAX_VERTICES / 3 + 1], ..Default::default() };
        assert_eq!(big.write_prbm(&mut out), Err(TooManyVertices((MAX_VERTICES / 3 + 1) * 3)));
    }
}
