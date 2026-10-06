//! Parsed hires tile. Vertex arrays are kept raw (lossless); triangles are 3 consecutive vertices.
//! Semantics per attribute: docs/03-rendering.md §2.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    pub material: u32,
    /// In vertices.
    pub start: u32,
    pub count: u32,
}

#[derive(Debug, Default)]
pub struct Tile {
    /// x/z relative to the tile's min corner, y absolute world y.
    pub position: Vec<[f32; 3]>,
    /// `(byte)(n*128-0.5)` of the geometric normal, same for all 3 vertices.
    pub normal: Vec<[i8; 3]>,
    /// Tint multiplier (255 = untinted), per face.
    pub color: Vec<[u8; 3]>,
    /// Texture-local, v=0 at the image top, spans one animation frame.
    pub uv: Vec<[f32; 2]>,
    /// Per vertex, 255 = unoccluded.
    pub ao: Vec<u8>,
    pub blocklight: Vec<i8>,
    pub sunlight: Vec<i8>,
    /// Sorted by material, covering every vertex exactly once.
    pub groups: Vec<Group>,
}

/// One triangle with its per-face values collapsed.
#[derive(Debug, Clone, Copy)]
pub struct Face {
    pub material: u32,
    pub pos: [[f32; 3]; 3],
    pub uv: [[f32; 2]; 3],
    pub ao: [u8; 3],
    pub color: [u8; 3],
    pub normal: [i8; 3],
    pub blocklight: i8,
    pub sunlight: i8,
}

impl Tile {
    pub fn face_count(&self) -> usize {
        self.position.len() / 3
    }

    pub fn faces(&self) -> impl Iterator<Item = Face> + '_ {
        self.groups.iter().flat_map(move |g| {
            let first = g.start as usize / 3;
            (first..first + g.count as usize / 3).map(move |i| self.face(i, g.material))
        })
    }

    fn face(&self, i: usize, material: u32) -> Face {
        let v = i * 3;
        Face {
            material,
            pos: [self.position[v], self.position[v + 1], self.position[v + 2]],
            uv: [self.uv[v], self.uv[v + 1], self.uv[v + 2]],
            ao: [self.ao[v], self.ao[v + 1], self.ao[v + 2]],
            color: self.color[v],
            normal: self.normal[v],
            blocklight: self.blocklight[v],
            sunlight: self.sunlight[v],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two triangles; per-vertex values are distinct so each can be traced to its slot.
    fn two_triangles() -> Tile {
        Tile {
            position: (0..6).map(|i| [i as f32, 64.0 + i as f32, -(i as f32)]).collect(),
            normal: vec![[0, 127, 0], [1, 1, 1], [1, 1, 1], [127, 0, 0], [2, 2, 2], [2, 2, 2]],
            color: vec![[255; 3], [0; 3], [0; 3], [10, 20, 30], [0; 3], [0; 3]],
            uv: (0..6).map(|i| [i as f32 / 8.0, 1.0]).collect(),
            ao: vec![255, 200, 100, 50, 25, 0],
            blocklight: vec![1, 9, 9, 3, 9, 9],
            sunlight: vec![15, 0, 0, -1, 0, 0],
            groups: vec![Group { material: 2, start: 0, count: 3 }, Group { material: 7, start: 3, count: 3 }],
        }
    }

    #[test]
    fn faces_follow_groups() {
        let tile = two_triangles();
        assert_eq!(tile.face_count(), 2);
        let faces: Vec<Face> = tile.faces().collect();
        assert_eq!(faces.iter().map(|f| f.material).collect::<Vec<_>>(), [2, 7]);
        assert_eq!(faces[1].pos, [[3.0, 67.0, -3.0], [4.0, 68.0, -4.0], [5.0, 69.0, -5.0]]);
        assert_eq!(faces[1].uv[2], [5.0 / 8.0, 1.0]);
        assert_eq!(faces[1].ao, [50, 25, 0]);
    }

    #[test]
    fn per_face_values_come_from_first_vertex() {
        let f = two_triangles().faces().nth(1).unwrap();
        assert_eq!((f.color, f.normal, f.blocklight, f.sunlight), ([10, 20, 30], [127, 0, 0], 3, -1));
    }

    #[test]
    fn empty_tile_has_no_faces() {
        let tile = Tile::default();
        assert_eq!(tile.face_count(), 0);
        assert_eq!(tile.faces().count(), 0);
    }
}
