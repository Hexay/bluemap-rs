//! The block cell of every quad, in the order the mesher leaves behind: inside a material group the block columns
//! come in scan order (x, then z) and each column top-down.
//!
//! Per quad `u8 step, i8 dy`: step = column index minus the previous quad's (zero at a group's first quad), column
//! index = `(x - x_min) * depth + (z - z_min)`; dy = y minus the previous quad's y, or, when the column changes or a
//! group starts, minus the y of the first quad of the previous column run. 255 / 127 escape to an i32 in the second
//! stream.

use super::bytes::{Reader, stream};
use super::{CELL_LIMIT, CompactError, Groups};

const STEP_ESCAPE: u8 = u8::MAX;
const DY_ESCAPE: i8 = i8::MAX;

#[derive(Clone, Copy)]
pub(super) struct Frame {
    pub x_min: i32,
    pub z_min: i32,
    pub depth: i32,
}

impl Frame {
    /// The frame of `cells`, which must be non-empty and within [`CELL_LIMIT`].
    pub fn of(cells: &[[i32; 3]]) -> Self {
        let min = |a: usize| cells.iter().map(|c| c[a]).min().unwrap_or(0);
        let z_max = cells.iter().map(|c| c[2]).max().unwrap_or(0);
        Self { x_min: min(0), z_min: min(2), depth: z_max - min(2) + 1 }
    }

    fn column(self, c: [i32; 3]) -> i64 {
        i64::from(c[0] - self.x_min) * i64::from(self.depth) + i64::from(c[2] - self.z_min)
    }
}

/// Predicts a quad's column and y from the quads before it.
struct Walk {
    column: i64,
    y: i64,
    run_top: i64,
}

impl Walk {
    /// `(column, y)` the quad's step and dy are relative to, given whether it starts a group.
    fn base(&self, group_start: bool) -> i64 {
        if group_start { 0 } else { self.column }
    }

    fn advance(&mut self, group_start: bool, column: i64, dy: impl FnOnce(i64) -> i64) -> i64 {
        let run_start = group_start || column != self.column;
        let y = dy(if run_start { self.run_top } else { self.y });
        if run_start {
            self.run_top = y;
        }
        (self.column, self.y) = (column, y);
        y
    }
}

pub(super) fn encode(body: &mut Vec<u8>, cells: &[[i32; 3]], group_quads: &[u32], frame: Frame) {
    let mut wide = Vec::new();
    stream(body, |out| {
        let mut groups = Groups::new(group_quads);
        let mut walk = Walk { column: 0, y: 0, run_top: 0 };
        for (q, &cell) in cells.iter().enumerate() {
            let start = groups.at(q).0;
            let column = frame.column(cell);
            let step = column - walk.base(start);
            let mut dy = 0;
            walk.advance(start, column, |base| {
                dy = i64::from(cell[1]) - base;
                i64::from(cell[1])
            });
            match u8::try_from(step).ok().filter(|&s| s != STEP_ESCAPE) {
                Some(s) => out.push(s),
                None => {
                    out.push(STEP_ESCAPE);
                    wide.extend((step as i32).to_le_bytes());
                }
            }
            match i8::try_from(dy).ok().filter(|&d| d != DY_ESCAPE) {
                Some(d) => out.push(d as u8),
                None => {
                    out.push(DY_ESCAPE as u8);
                    wide.extend((dy as i32).to_le_bytes());
                }
            }
        }
    });
    stream(body, |out| out.extend(&wide));
}

pub(super) fn decode(
    r: &mut Reader,
    quads: usize,
    group_quads: &[u32],
    frame: Frame,
    cells: &mut Vec<[i32; 3]>,
) -> Result<(), CompactError> {
    let mut steps = r.stream(Some(quads * 2))?;
    let mut wide = r.stream(None)?;
    if frame.depth <= 0 {
        return Err(CompactError::Corrupt("cell frame"));
    }
    let depth = i64::from(frame.depth);
    let limit = i64::from(CELL_LIMIT);
    let mut groups = Groups::new(group_quads);
    let mut walk = Walk { column: 0, y: 0, run_top: 0 };
    cells.clear();
    cells.reserve(quads);
    for q in 0..quads {
        let start = groups.at(q).0;
        let (step, dy) = (steps.u8()?, steps.u8()? as i8);
        let mut value = |escaped: bool, small: i64| -> Result<i64, CompactError> {
            Ok(if escaped { i64::from(wide.u32()? as i32) } else { small })
        };
        let step = value(step == STEP_ESCAPE, i64::from(step))?;
        let dy = value(dy == DY_ESCAPE, i64::from(dy))?;
        let column = walk.base(start) + step;
        let y = walk.advance(start, column, |base| base + dy);
        let (x, z) = (i64::from(frame.x_min) + column.div_euclid(depth), i64::from(frame.z_min) + column % depth);
        if column < 0 || [x, y, z].iter().any(|v| v.abs() > limit) {
            return Err(CompactError::Corrupt("cell"));
        }
        cells.push([x as i32, y as i32, z as i32]);
    }
    if wide.is_empty() { Ok(()) } else { Err(CompactError::Corrupt("cell escapes")) }
}
