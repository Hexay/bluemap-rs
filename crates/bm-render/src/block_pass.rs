//! `BlockRenderPass` and `BlockStateModelRenderer`: every block of the tile's columns, top down, plus the column
//! colour, height and light the lowres layer is fed.

use bm_format::prbm::TileModel;
use bm_math::Color;
use bm_resources::blockstate::RendererType;
use bm_resources::model::Direction;

use crate::context::{Block, Ctx};
use crate::flags::Hidden;
use crate::mesh::{CapacityReached, MeshExt};
use crate::relative::Offset;
use crate::states::StateInfo;
use crate::{ColumnMeta, liquid, resource};

/// Renders blocks x in `min[0]..=max[0]`, z in `min[1]..=max[1]` into `out` (positions relative to `min`),
/// pushing one [`ColumnMeta`] per column, x-major. Stops early, like BlueMap, when the model is full.
pub(crate) fn render(
    ctx: &Ctx,
    min: [i32; 2],
    max: [i32; 2],
    out: &mut TileModel,
    columns: &mut Vec<ColumnMeta>,
) -> Result<(), CapacityReached> {
    let mut block_color = Color::default();
    for x in min[0]..=max[0] {
        for z in min[1]..=max[1] {
            let mut max_height = i32::MIN;
            let mut top_block_light = 0.0f64;
            let mut column_color = Color { premultiplied: true, ..Color::default() };

            if ctx.view.inside_column(x, z) {
                let (min_y, max_y) = ctx.view.column_y_range(x, z);
                // the volume spans every column's y range plus a border, so the whole column is interior or none of it
                let top = ctx.view.interior_index(x, max_y, z);
                let mut light_under = |block_light: u8, column: &Color| {
                    top_block_light = top_block_light.max(f64::from(f32::from(block_light) * (1.0 - column.a)));
                };
                for y in (min_y..=max_y).rev() {
                    if !ctx.view.inside(x, y, z) {
                        continue;
                    }
                    let index = top.map(|t| t - (max_y - y) as usize);
                    // air renders nothing and leaves the colour alone; only its light reaches the column
                    if let Some(i) = index.filter(|&i| ctx.states.flags(ctx.view.state_at(i)).is_air()) {
                        light_under(ctx.view.light_at(i).1, &column_color);
                        continue;
                    }
                    // the same for buried blocks: every cullface culled means no faces and no colour
                    if let Some(i) = index.filter(|&i| fully_culled(ctx, i)) {
                        debug_assert!(renders_nothing(ctx, &Block::new(ctx, x, y, z, index)));
                        light_under(ctx.view.light_at(i).1, &column_color);
                        continue;
                    }
                    let block = Block::new(ctx, x, y, z, index);
                    let start = out.faces();
                    render_block(ctx, &block, out, &mut block_color)?;
                    light_under(block.block_light, &column_color);
                    out.translate_from(start, (x - min[0]) as f32, y as f32, (z - min[1]) as f32);

                    if block_color.a > 0.0 {
                        max_height = max_height.max(y);
                        column_color.underlay(block_color.premultiplied());
                    }
                    if ctx.settings.render_top_only && f64::from(block_color.a) > 0.999 && block.info.props.culling {
                        break;
                    }
                }
            }

            let height = if max_height == i32::MIN { 0 } else { max_height };
            columns.push(ColumnMeta { x, z, color: column_color, height, block_light: top_block_light as i32 });
        }
    }
    Ok(())
}

/// Whether the interior block at volume index `i` is [`Hidden`] by its neighbours.
fn fully_culled(ctx: &Ctx, i: usize) -> bool {
    let id = ctx.view.state_at(i);
    let neighbor = |slot: u8| ctx.view.state_at(ctx.view.step(i, slot));
    match ctx.states.get(id).hidden {
        Hidden::Never => false,
        Hidden::Cullfaces(mut slots) => {
            while slots != 0 {
                let slot = slots.trailing_zeros() as u8;
                slots &= slots - 1;
                if !ctx.states.culls(neighbor(slot), id) {
                    return false;
                }
            }
            true
        }
        Hidden::Water => Direction::ALL.iter().all(|&dir| {
            let [x, y, z] = dir.to_vector();
            let flags = ctx.states.flags(neighbor(Offset::new(x, y, z).slot));
            flags.watery() || (dir != Direction::Up && flags.culling())
        }),
    }
}

/// The full render of a block [`fully_culled`] skips, for debug builds to check it adds nothing.
fn renders_nothing(ctx: &Ctx, block: &Block) -> bool {
    let (mut out, mut color) = (TileModel::default(), Color::default());
    render_block(ctx, block, &mut out, &mut color).is_ok() && out.faces() == 0 && color.a == 0.0
}

/// `BlockStateModelRenderer.render`: the block's variants, then water if it's waterlogged.
fn render_block(ctx: &Ctx, block: &Block, out: &mut TileModel, color: &mut Color) -> Result<(), CapacityReached> {
    color.set(0.0, 0.0, 0.0, 0.0, true);
    if block.info.is_air() {
        return Ok(());
    }
    render_model(ctx, block, block.info, out, color)?;
    if block.info.renders_water() {
        let mut water = Color { premultiplied: true, ..Color::default() };
        render_model(ctx, block, ctx.states.get(ctx.states.water), out, &mut water)?;
        *color = *water.overlay(color.premultiplied());
    }
    Ok(())
}

/// `renderModel`: the variants `state` resolves to at the block's position, rendered as the block.
fn render_model(
    ctx: &Ctx,
    block: &Block,
    state: &StateInfo,
    out: &mut TileModel,
    color: &mut Color,
) -> Result<(), CapacityReached> {
    let mut opacity = 0.0f32;
    for set in &state.sets {
        let Some(v) = set.pick(block.x, block.y, block.z) else { continue };
        let mut variant_color = Color { premultiplied: true, ..Color::default() };
        match v.renderer {
            RendererType::Liquid => liquid::render(ctx, block, v, out, &mut variant_color)?,
            _ => resource::render(ctx, block, v, out, &mut variant_color)?,
        }
        if variant_color.a > opacity {
            opacity = variant_color.a;
        }
        color.add(variant_color.premultiplied());
    }
    if color.a > 0.0 {
        color.flatten().straight();
        color.a = opacity;
    }
    Ok(())
}
