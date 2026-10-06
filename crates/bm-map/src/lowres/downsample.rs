use bm_format::lowres::LowresTile;
use bm_math::Color;

/// `LowresLayer.saveTile`'s next-LOD pass: one pixel per `factor × factor` group of the tile core (seams excluded).
/// Colour is the mean of premultiplied colours, re-straightened and truncated; height and light are truncating
/// integer means. `emit(gx, gz, argb, height, block_light)` gets each group in Java's order.
pub(super) fn downsample<E>(
    tile: &LowresTile,
    size: [i32; 2],
    factor: i32,
    mut emit: impl FnMut(i32, i32, u32, i32, i32) -> Result<(), E>,
) -> Result<(), E> {
    let groups = [size[0].div_euclid(factor), size[1].div_euclid(factor)];
    let (mut avg, mut color) = (Color::default(), Color::default());
    for gx in 0..groups[0] {
        for gz in 0..groups[1] {
            avg.set(0.0, 0.0, 0.0, 0.0, true);
            let (mut height, mut light, mut count) = (0i32, 0i32, 0i32);
            // x outer, z inner: the float sum must accumulate in Java's order
            for x in 0..factor {
                for z in 0..factor {
                    let (px, pz) = ((gx * factor + x) as usize, (gz * factor + z) as usize);
                    count += 1;
                    avg.add(color.set_int(tile.color(px, pz) as i32).premultiplied());
                    height += tile.height(px, pz);
                    light += i32::from(tile.block_light(px, pz));
                }
            }
            avg.div(count);
            emit(gx, gz, avg.straight().get_int() as u32, height / count, light / count)?;
        }
    }
    Ok(())
}
