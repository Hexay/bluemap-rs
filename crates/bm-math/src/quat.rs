//! The rotation quaternions BlueMap builds (identically) in `MatrixM3f`, `MatrixM4f` and `ArrayTileModel`, as
//! `[x, y, z, w]` doubles. The matrices narrow them to float; `ArrayTileModel` rotates with the doubles.
//! Angles are degrees; sines and cosines come from flow-math's table, widened from float.

use bm_java::math::to_radians;
use bm_java::trig::{cos, sin};

fn half_sin_cos(deg: f32) -> (f64, f64) {
    let half = to_radians(deg as f64) * 0.5;
    (sin(half) as f64, cos(half) as f64)
}

/// `rotate(angle, axisX, axisY, axisZ)`: normalised axis-angle quaternion.
pub fn axis_angle(angle: f32, axis_x: f32, axis_y: f32, axis_z: f32) -> [f64; 4] {
    let half = to_radians(angle as f64) * 0.5;
    // the axis length is summed in float before the double sqrt
    let q = sin(half) as f64 / ((axis_x * axis_x + axis_y * axis_y + axis_z * axis_z) as f64).sqrt();
    let qx = axis_x as f64 * q;
    let qy = axis_y as f64 * q;
    let qz = axis_z as f64 * q;
    let qw = cos(half) as f64;
    let len = (qx * qx + qy * qy + qz * qz + qw * qw).sqrt();
    [qx / len, qy / len, qz / len, qw / len]
}

pub fn euler_xyz(pitch: f32, yaw: f32, roll: f32) -> [f64; 4] {
    let (sx, cx) = half_sin_cos(pitch);
    let (sy, cy) = half_sin_cos(yaw);
    let (sz, cz) = half_sin_cos(roll);
    let (cycz, sysz, sycz, cysz) = (cy * cz, sy * sz, sy * cz, cy * sz);
    [sx * cycz + cx * sysz, cx * sycz - sx * cysz, cx * cysz + sx * sycz, cx * cycz - sx * sysz]
}

pub fn euler_zyx(pitch: f32, yaw: f32, roll: f32) -> [f64; 4] {
    let (sx, cx) = half_sin_cos(pitch);
    let (sy, cy) = half_sin_cos(yaw);
    let (sz, cz) = half_sin_cos(roll);
    let (cycz, sysz, sycz, cysz) = (cy * cz, sy * sz, sy * cz, cy * sz);
    [cx * cycz + sx * sysz, sx * cycz - cx * sysz, cx * sycz + sx * cysz, cx * cysz - sx * sycz]
}

/// The order block-state variants, model element rotations and entity models use.
pub fn euler_yxz(pitch: f32, yaw: f32, roll: f32) -> [f64; 4] {
    let (sx, cx) = half_sin_cos(pitch);
    let (sy, cy) = half_sin_cos(yaw);
    let (sz, cz) = half_sin_cos(roll);
    let (cysx, sycx, sysx, cycx) = (cy * sx, sy * cx, sy * sx, cy * cx);
    [cysx * cz + sycx * sz, sycx * cz - cysx * sz, cycx * sz - sysx * cz, cycx * cz + sysx * sz]
}

pub(crate) fn to_f32(q: [f64; 4]) -> [f32; 4] {
    q.map(|c| c as f32)
}

/// The quaternion's rotation matrix, row-major, as both matrix types build it.
pub(crate) fn rotation_matrix([qx, qy, qz, qw]: [f32; 4]) -> [f32; 9] {
    [
        1.0 - 2.0 * qy * qy - 2.0 * qz * qz,
        2.0 * qx * qy - 2.0 * qw * qz,
        2.0 * qx * qz + 2.0 * qw * qy,
        2.0 * qx * qy + 2.0 * qw * qz,
        1.0 - 2.0 * qx * qx - 2.0 * qz * qz,
        2.0 * qy * qz - 2.0 * qw * qx,
        2.0 * qx * qz - 2.0 * qw * qy,
        2.0 * qy * qz + 2.0 * qx * qw,
        1.0 - 2.0 * qx * qx - 2.0 * qy * qy,
    ]
}
