//! `LiquidModelRenderer`: water and lava as a box with sloped top corners and flowing UVs.

use std::cell::OnceCell;

use bm_format::prbm::TileModel;
use bm_java::trig::RAD_TO_DEG;
use bm_math::{Color, MatrixM3f, VectorM2f};
use bm_resources::model::Direction;

use crate::context::{Block, Ctx};
use crate::mesh::{CapacityReached, Face, MeshExt};
use crate::relative::Offset;
use crate::states::{StateInfo, VariantInfo};

const BLOCK_SCALE: f32 = 1.0 / 16.0;

pub(crate) fn render(
    ctx: &Ctx,
    block: &Block,
    v: &VariantInfo,
    out: &mut TileModel,
    color: &mut Color,
) -> Result<(), CapacityReached> {
    if v.model.is_none() {
        return Ok(());
    }
    // waterlogged blocks render as plain water
    let liquid = if block.info.renders_water() { ctx.states.get(ctx.states.water) } else { block.info };
    Liquid { ctx, block, v, liquid }.build(out, color)
}

struct Liquid<'x, 'r, 'a> {
    ctx: &'x Ctx<'r, 'a>,
    block: &'x Block<'r, 'a>,
    v: &'x VariantInfo<'a>,
    liquid: &'x StateInfo<'a>,
}

impl Liquid<'_, '_, '_> {
    fn build(&self, out: &mut TileModel, color: &mut Color) -> Result<(), CapacityReached> {
        let (ctx, block) = (self.ctx, self.block);
        let (sun, block_light) = (block.sky, block.block_light);
        if block.culled_as_cave(ctx, sun, block_light) {
            return Ok(());
        }

        let level = self.liquid.liquid_level;
        let top = if level < 8 && !(level == 0 && self.same_liquid(block.neighbor(ctx, Offset::new(0, 1, 0)))) {
            [self.corner_height(-1, -1), self.corner_height(-1, 0), self.corner_height(0, -1), self.corner_height(0, 0)]
        } else {
            [16.0; 4]
        };
        let c = [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 16.0],
            [16.0, 0.0, 0.0],
            [16.0, 0.0, 16.0],
            [0.0, top[0], 0.0],
            [0.0, top[1], 16.0],
            [16.0, top[2], 0.0],
            [16.0, top[3], 16.0],
        ];
        // Java blends the tint up front; it only shows once a face survives culling, which most water doesn't
        let tint = OnceCell::new();

        let start = out.faces();
        let mut face = |dir, k: [usize; 4]| self.face(dir, k.map(|k| c[k]), &tint, out);
        face(Direction::Down, [0, 2, 3, 1])?;
        let up_rendered = face(Direction::Up, [5, 7, 6, 4])?;
        face(Direction::North, [2, 0, 4, 6])?;
        face(Direction::South, [1, 3, 7, 5])?;
        face(Direction::West, [0, 1, 5, 4])?;
        face(Direction::East, [3, 2, 6, 7])?;
        out.scale_from(start, BLOCK_SCALE);

        match self.v.still.color.filter(|_| up_rendered) {
            Some(texture) => {
                *color = texture;
                color.multiply(self.tint(&tint));
                let ambient = ctx.settings.ambient_light;
                let light = f32::from(sun.max(block_light)) / 15.0;
                let light = (ambient + light) / (ambient + 1.0);
                color.r *= light;
                color.g *= light;
                color.b *= light;
            }
            None if !up_rendered => {
                color.set(0.0, 0.0, 0.0, 0.0, true);
            }
            None => {}
        }
        Ok(())
    }

    fn tint<'c>(&self, cell: &'c OnceCell<Color>) -> &'c Color {
        cell.get_or_init(|| self.ctx.tint(self.liquid, self.block.x, self.block.y, self.block.z))
    }

    fn same_liquid(&self, other: &StateInfo) -> bool {
        if self.liquid.is_water() {
            return other.is_water() || other.renders_water();
        }
        other.state.name == self.liquid.state.name
    }

    fn base_height(state: &StateInfo) -> f32 {
        let level = state.liquid_level;
        if level >= 8 { 16.0 } else { 14.0 - level as f32 * 1.9 }
    }

    /// `getLiquidCornerHeight`, in 0..16 units.
    fn corner_height(&self, x: i32, z: i32) -> f32 {
        let (ctx, block) = (self.ctx, self.block);
        for ix in x..=x + 1 {
            for iz in z..=z + 1 {
                if self.same_liquid(block.neighbor(ctx, Offset::new(ix, 1, iz))) {
                    return 16.0;
                }
            }
        }
        let (mut sum, mut count) = (0.0f32, 0);
        for ix in x..=x + 1 {
            for iz in z..=z + 1 {
                let neighbor = block.neighbor(ctx, Offset::new(ix, 0, iz));
                if self.same_liquid(neighbor) {
                    if neighbor.liquid_level == 0 {
                        return 14.0;
                    }
                    sum += Self::base_height(neighbor);
                    count += 1;
                } else if neighbor.is_air() {
                    count += 1;
                }
            }
        }
        if sum == 0.0 || count == 0 { 3.0 } else { sum / count as f32 }
    }

    /// Whether the face was rendered.
    fn face(
        &self,
        dir: Direction,
        c: [[f32; 3]; 4],
        tint: &OnceCell<Color>,
        out: &mut TileModel,
    ) -> Result<bool, CapacityReached> {
        let (ctx, block) = (self.ctx, self.block);
        let [dx, dy, dz] = dir.to_vector();
        let to_neighbor = Offset::new(dx, dy, dz);
        let neighbor = block.neighbor(ctx, to_neighbor);
        if self.same_liquid(neighbor) || (dir != Direction::Up && neighbor.props.culling) {
            return Ok(false);
        }
        out.reserve_faces(2)?;
        let tint = self.tint(tint);

        let mut uvs =
            [VectorM2f::new(0.0, 1.0), VectorM2f::new(1.0, 1.0), VectorM2f::new(1.0, 0.0), VectorM2f::new(0.0, 0.0)];
        let mut flow = false;
        if dir == Direction::Up {
            if let Some(angle) = self.flowing_angle() {
                flow = true;
                let mut t = MatrixM3f::IDENTITY;
                t.translate(-0.5, -0.5).scale(0.5, 0.5, 1.0).rotate(-angle as f32, 0.0, 0.0, 1.0).translate(0.5, 0.5);
                uvs.iter_mut().for_each(|uv| _ = uv.transform(&t));
            }
        } else if dir != Direction::Down {
            flow = true;
            let mut t = MatrixM3f::IDENTITY;
            t.translate(-0.5, -0.5).scale(0.5, 0.5, 1.0).translate(0.5, 0.5);
            uvs.iter_mut().for_each(|uv| _ = uv.transform(&t));
        }

        let (sky, block_light) =
            if dir == Direction::Up { (block.sky, block.block_light) } else { block.neighbor_light(ctx, to_neighbor) };
        let material = if flow { self.v.flow.id } else { self.v.still.id };
        let uv = uvs.map(|v| [v.x, v.y]);
        let tri = |a: usize, b: usize, d: usize| Face {
            positions: [c[a], c[b], c[d]],
            uvs: [uv[a], uv[b], uv[d]],
            aos: [1.0; 3],
            color: [tint.r, tint.g, tint.b],
            sunlight: i32::from(sky),
            blocklight: i32::from(block_light),
            material,
        };
        out.push_face(&tri(0, 1, 2));
        out.push_face(&tri(0, 2, 3));
        Ok(true)
    }

    /// `getFlowingAngle` in whole degrees; `None` when still.
    fn flowing_angle(&self) -> Option<i32> {
        let own = Self::base_height(self.liquid) * BLOCK_SCALE;
        if f64::from(own) > 0.8 {
            return None;
        }
        let mut flow = VectorM2f::default();
        flow.x += self.compare_heights(own, -1, 0);
        flow.x -= self.compare_heights(own, 1, 0);
        flow.y -= self.compare_heights(own, 0, -1);
        flow.y += self.compare_heights(own, 0, 1);
        if flow.x == 0.0 && flow.y == 0.0 {
            return None;
        }
        let angle = (f64::from(flow.angle_to(0.0, -1.0)) * RAD_TO_DEG) as i32;
        Some(if flow.x < 0.0 { angle } else { -angle })
    }

    fn compare_heights(&self, own: f32, dx: i32, dz: i32) -> f32 {
        let neighbor = self.block.neighbor(self.ctx, Offset::new(dx, 0, dz));
        if neighbor.is_air() || !self.same_liquid(neighbor) {
            return 0.0;
        }
        Self::base_height(neighbor) * BLOCK_SCALE - own
    }
}
