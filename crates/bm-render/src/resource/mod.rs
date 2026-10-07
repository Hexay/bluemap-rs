//! `ResourceModelRenderer`: a block-model variant as faces, with culling, light, AO, tint and the map colour.

mod geometry;

use bm_format::prbm::TileModel;
use bm_java::trig;
use bm_math::{Color, VectorM2f, VectorM3f};
use bm_resources::model::{BakedElement, BakedModel, Direction};

use crate::context::{Block, Ctx};
use crate::mesh::{CapacityReached, Face, MeshExt};
use crate::states::VariantInfo;

/// Renders one variant into `out` and returns its map colour contribution in `color`.
pub(crate) fn render(
    ctx: &Ctx,
    block: &Block,
    v: &VariantInfo,
    out: &mut TileModel,
    color: &mut Color,
) -> Result<(), CapacityReached> {
    let Some(model) = v.model else { return Ok(()) };
    let mut r = Renderer { ctx, block, v, model, tint: None, opacity: 0.0 };
    let start = out.faces();
    for (i, element) in model.elements.iter().enumerate() {
        let element_start = out.faces();
        r.element(i, element, out, color)?;
        out.transform_from(element_start, &element.transform);
    }
    if color.a > 0.0 {
        color.flatten().straight();
        color.a = r.opacity;
    }
    if v.variant.transformed {
        out.transform_from(start, &v.variant.transform);
    }
    if block.info.props.random_offset {
        let dx = (geometry::hash_to_float(block.x, block.z, 123984) - 0.5) * 0.75;
        let dz = (geometry::hash_to_float(block.x, block.z, 345542) - 0.5) * 0.75;
        out.translate_from(start, dx, 0.0, dz);
    }
    Ok(())
}

struct Renderer<'x, 'r, 'a> {
    ctx: &'x Ctx<'r, 'a>,
    block: &'x Block<'r, 'a>,
    v: &'x VariantInfo<'a>,
    model: &'a BakedModel,
    /// Computed at the first tinted face, then also applied to every later map colour of this variant.
    tint: Option<Color>,
    opacity: f32,
}

type Corner = [f32; 3];

impl Renderer<'_, '_, '_> {
    fn element(
        &mut self,
        i: usize,
        e: &BakedElement,
        out: &mut TileModel,
        color: &mut Color,
    ) -> Result<(), CapacityReached> {
        let ([fx, fy, fz], [tx, ty, tz]) = (e.from, e.to);
        let c = [
            [fx, fy, fz],
            [fx, fy, tz],
            [tx, fy, fz],
            [tx, fy, tz],
            [fx, ty, fz],
            [fx, ty, tz],
            [tx, ty, fz],
            [tx, ty, tz],
        ];
        let mut face = |dir, corners: [usize; 4]| self.face(i, e, dir, corners.map(|k| c[k]), out, color);
        face(Direction::Down, [0, 2, 3, 1])?;
        face(Direction::Up, [5, 7, 6, 4])?;
        face(Direction::North, [2, 0, 4, 6])?;
        face(Direction::South, [1, 3, 7, 5])?;
        face(Direction::West, [0, 1, 5, 4])?;
        face(Direction::East, [3, 2, 6, 7])
    }

    /// `createElementFace`.
    fn face(
        &mut self,
        element_index: usize,
        e: &BakedElement,
        dir: Direction,
        c: [Corner; 4],
        out: &mut TileModel,
        color: &mut Color,
    ) -> Result<(), CapacityReached> {
        let Some(face) = e.face(dir) else { return Ok(()) };
        let (ctx, block) = (self.ctx, self.block);

        // Java tests light and caves first; every test is side-effect free, so the cheapest goes first
        if let Some(cull) = face.cullface {
            let [cx, cy, cz] = self.v.relative.get(cull.to_vector());
            let (id, info) = block.neighbor(ctx, cx, cy, cz);
            if info.props.culling || (info.props.culling_identical && id == block.id) {
                return Ok(());
            }
        }
        let mut facing = VectorM3f::default();
        facing.set_i(dir.to_vector());
        facing.rotate_and_scale(&e.rotation);
        self.make_relative(&mut facing);
        if ctx.settings.render_top_only && f64::from(facing.y) < 0.01 {
            return Ok(());
        }

        let [nx, ny, nz] = self.v.relative.get(dir.to_vector());
        let (sky, block_light) = block.neighbor_light(ctx, nx, ny, nz);
        let sun = block.sky.max(sky);
        let block_light = block.block_light.max(block_light);
        if block.culled_as_cave(ctx, sun, block_light) {
            return Ok(());
        }

        out.reserve_faces(2)?;
        let material = self.v.materials[element_index][dir.index()];

        let [u0, v0, u1, v1] = face.uv.map(|c| c / 16.0);
        let raw = [VectorM2f::new(u0, v1), VectorM2f::new(u1, v1), VectorM2f::new(u1, v0), VectorM2f::new(u0, v0)];
        let steps = face.uv_rotation_steps();
        let mut uvs: [VectorM2f; 4] = std::array::from_fn(|i| raw[(steps + i) % 4]);
        let variant = self.v.variant;
        if variant.uvlock && variant.transformed {
            let rotation = f64::from(self.uv_lock_rotation(dir));
            let (cos, sin) = (trig::cos(rotation), trig::sin(rotation));
            for uv in &mut uvs {
                uv.translate(-0.5, -0.5).rotate(cos, sin).translate(0.5, 0.5);
            }
        }

        let tint = if face.tinted {
            let t = *self.tint.get_or_insert_with(|| ctx.tint(block.info, block.x, block.y, block.z));
            [t.r, t.g, t.b]
        } else {
            [1.0; 3]
        };
        let blocklight = i32::from(block_light).max(e.light_emission);

        let ao = if self.model.ambient_occlusion { c.map(|corner| self.ao(corner, dir)) } else { [1.0; 4] };
        let uv = uvs.map(|v| [v.x, v.y]);
        let tri = |a: usize, b: usize, d: usize| Face {
            positions: [c[a], c[b], c[d]],
            uvs: [uv[a], uv[b], uv[d]],
            aos: [ao[a], ao[b], ao[d]],
            color: tint,
            sunlight: i32::from(sun),
            blocklight,
            material: material.id,
        };
        let (first, second) = (tri(0, 1, 2), tri(0, 2, 3));
        out.push_face(&first);
        out.push_face(&second);

        if f64::from(facing.y) > 0.01
            && let Some(texture) = material.color
        {
            self.add_map_color(texture, sun, block_light, color);
        }
        Ok(())
    }

    fn add_map_color(&mut self, texture: Color, sun: u8, block_light: u8, color: &mut Color) {
        let mut c = texture;
        if let Some(tint) = &self.tint {
            c.multiply(tint);
        }
        let ambient = self.ctx.settings.ambient_light;
        let light = (f32::from(sun) / 15.0).max(f32::from(block_light) / 15.0);
        let light = (1.0 - ambient) * light + ambient;
        c.r *= light;
        c.g *= light;
        c.b *= light;
        if c.a > self.opacity {
            self.opacity = c.a;
        }
        color.add(&c);
    }
}
