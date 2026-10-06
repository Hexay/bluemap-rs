use super::*;
use crate::Group;

/// Up-facing triangle on top of block (x, y, z), tile-relative.
fn top_face(x: f32, y: f32, z: f32) -> [[f32; 3]; 3] {
    [[x, y + 1.0, z], [x + 1.0, y + 1.0, z], [x, y + 1.0, z + 1.0]]
}

/// One material per face; `ao` and `blocklight` apply to all of a face's vertices.
fn tile(faces: &[([[f32; 3]; 3], u8, i8)]) -> Tile {
    let mut t = Tile::default();
    for (i, &(pos, ao, light)) in faces.iter().enumerate() {
        t.position.extend(pos);
        t.normal.extend([[0, 127, 0]; 3]);
        t.color.extend([[255; 3]; 3]);
        t.uv.extend([[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        t.ao.extend([ao; 3]);
        t.blocklight.extend([light; 3]);
        t.sunlight.extend([15; 3]);
        t.groups.push(Group { material: i as u32, start: 3 * i as u32, count: 3 });
    }
    t
}

fn names(n: &[&str]) -> Vec<String> {
    n.iter().map(|s| s.to_string()).collect()
}

#[test]
fn identical_renders_pair_every_face() {
    let t = tile(&[(top_face(0.0, 64.0, 0.0), 255, 0), (top_face(1.0, 64.0, 0.0), 255, 0)]);
    let n = names(&["stone", "dirt"]);
    let mut d = RenderDiff::default();
    d.add_tile(&t, &n, &t, &n, [0, 0], |_, _| false);
    let r = d.finish(5);
    assert_eq!((r.original_faces, r.paired, r.identical), (2, 2, 2));
    assert!(r.missing.is_empty() && r.extra.is_empty() && r.cells.is_empty());
}

#[test]
fn vertex_order_does_not_matter() {
    let [a, b, c] = top_face(0.0, 64.0, 0.0);
    let mut rotated = tile(&[([b, c, a], 255, 0)]);
    rotated.uv.rotate_left(1);
    let n = names(&["stone"]);
    let mut d = RenderDiff::default();
    d.add_tile(&tile(&[([a, b, c], 255, 0)]), &n, &rotated, &n, [0, 0], |_, _| false);
    assert_eq!(d.finish(5).identical, 1);
}

#[test]
fn texture_and_light_differences_are_attributed_to_the_cell() {
    let t = |light| tile(&[(top_face(2.0, 64.0, 3.0), 200, light)]);
    let mut d = RenderDiff::default();
    d.add_tile(&t(14), &names(&["torch_lit_floor"]), &t(10), &names(&["stone"]), [16, -32], |_, _| false);
    let r = d.finish(5);
    assert_eq!((r.paired, r.identical), (1, 0));
    assert_eq!(r.aspects["texture"].faces, 1);
    assert_eq!(r.aspects["blocklight"].mean_delta, Some(4.0));
    assert_eq!(r.aspects["ao"].faces, 0);
    assert_eq!(r.cells[0].pos, [18, 64, -29]);
    assert_eq!(r.cells[0].aspects, ["texture", "blocklight"]);
}

#[test]
fn unpaired_faces_count_as_missing_and_extra() {
    let original = tile(&[(top_face(0.0, 64.0, 0.0), 255, 0), (top_face(0.0, 70.0, 0.0), 255, 0)]);
    let candidate = tile(&[(top_face(0.0, 64.0, 0.0), 255, 0), (top_face(5.0, 64.0, 0.0), 255, 0)]);
    let n = names(&["grass", "leaves"]);
    let mut d = RenderDiff::default();
    d.add_tile(&original, &n, &candidate, &n, [0, 0], |_, _| false);
    let r = d.finish(5);
    assert_eq!((r.paired, r.identical), (1, 1));
    assert_eq!(r.missing, [("leaves".to_string(), 1)]);
    assert_eq!(r.extra, [("leaves".to_string(), 1)]);
    assert_eq!(r.cells.len(), 2);
}

#[test]
fn faces_looking_out_of_the_render_are_skipped() {
    let mut wall = tile(&[(top_face(0.0, 64.0, 0.0), 255, 0)]);
    wall.normal = vec![[-127, 0, 0]; 3];
    let n = names(&["stone"]);
    let mut d = RenderDiff::default();
    d.add_tile(&Tile::default(), &n, &wall, &n, [0, 0], |x, _| x < 0);
    let r = d.finish(5);
    assert_eq!((r.edge_faces, r.candidate_faces), (1, 0));
    assert!(r.extra.is_empty());
}

#[test]
fn absent_tile_is_all_missing() {
    let t = tile(&[(top_face(0.0, 64.0, 0.0), 255, 0)]);
    let mut d = RenderDiff::default();
    d.add_tile(&t, &names(&["stone"]), &Tile::default(), &[], [0, 0], |_, _| false);
    let r = d.finish(5);
    assert_eq!((r.original_faces, r.candidate_faces, r.paired), (1, 0, 0));
    assert_eq!(r.missing, [("stone".to_string(), 1)]);
}
