//! Synthetic PRBM buffers in the byte layout PRBMWriter.java writes.

pub struct Prbm {
    pub version: u8,
    /// OR-ed into the flags byte next to the attribute count.
    pub extra_flags: u8,
    pub vertices: u32,
    /// (name, attribute flags, raw values)
    pub attrs: Vec<(&'static str, u8, Vec<u8>)>,
    /// (material, start, count); the -1 terminator is appended.
    pub groups: Vec<[i32; 3]>,
    pub trailer: Vec<u8>,
}

fn pad(b: &mut Vec<u8>) {
    while !b.len().is_multiple_of(4) {
        b.push(0);
    }
}

fn f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

impl Prbm {
    /// One triangle, material 5.
    pub fn one_triangle() -> Self {
        Self {
            version: 1,
            extra_flags: 0,
            vertices: 3,
            attrs: vec![
                ("position", 0x21, f32s(&[0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0])),
                ("normal", 0x63, [0, 127, 0].repeat(3)),
                ("color", 0x67, [255, 128, 0].repeat(3)),
                ("uv", 0x11, f32s(&[0.0, 0.0, 1.0, 0.0, 1.0, 1.0])),
                ("ao", 0x47, vec![255, 191, 127]),
                ("blocklight", 0x03, vec![14; 3]),
                ("sunlight", 0x03, vec![15; 3]),
            ],
            groups: vec![[5, 0, 3]],
            trailer: Vec::new(),
        }
    }

    pub fn attr_mut(&mut self, name: &str) -> &mut (&'static str, u8, Vec<u8>) {
        self.attrs.iter_mut().find(|a| a.0 == name).expect("attribute")
    }

    pub fn bytes(&self) -> Vec<u8> {
        let mut b = vec![self.version, self.extra_flags | self.attrs.len() as u8];
        b.extend(&self.vertices.to_le_bytes()[..3]);
        b.extend([0; 3]);
        for (name, flags, values) in &self.attrs {
            b.extend(name.as_bytes());
            b.extend([0, *flags]);
            pad(&mut b);
            b.extend(values);
        }
        pad(&mut b);
        for v in self.groups.iter().flatten().chain(&[-1]) {
            b.extend(v.to_le_bytes());
        }
        b.extend(&self.trailer);
        b
    }
}
