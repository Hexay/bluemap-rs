//! Primitive masks: `BoxMask`, `EllipseMask`, `PolygonMask`.

use std::sync::Arc;

use super::Tristate;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoxMask {
    pub min: [i32; 3],
    pub max: [i32; 3],
}

impl BoxMask {
    pub fn test(&self, x: i32, y: i32, z: i32) -> bool {
        self.test_xz(x, z) && y >= self.min[1] && y <= self.max[1]
    }

    pub fn test_xz(&self, x: i32, z: i32) -> bool {
        x >= self.min[0] && x <= self.max[0] && z >= self.min[2] && z <= self.max[2]
    }

    pub fn test_area(&self, min_x: i32, min_y: i32, min_z: i32, max_x: i32, max_y: i32, max_z: i32) -> Tristate {
        let (min, max) = (self.min, self.max);
        if min_x >= min[0]
            && max_x <= max[0]
            && min_z >= min[2]
            && max_z <= max[2]
            && min_y >= min[1]
            && max_y <= max[1]
        {
            return Tristate::True;
        }
        if max_x < min[0] || min_x > max[0] || max_z < min[2] || min_z > max[2] || max_y < min[1] || min_y > max[1] {
            return Tristate::False;
        }
        Tristate::Undefined
    }

    pub fn is_edge(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> bool {
        self.test_area(min_x, self.min[1], min_z, max_x, self.max[1], max_z) == Tristate::Undefined
    }
}

fn test_y(min_y: i32, max_y: i32, mask_min_y: i32, mask_max_y: i32) -> Tristate {
    if max_y < mask_min_y || min_y > mask_max_y {
        Tristate::False
    } else if min_y >= mask_min_y && max_y <= mask_max_y {
        Tristate::True
    } else {
        Tristate::Undefined
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EllipseMask {
    pub center: [f64; 2],
    radius_squared_x: f64,
    radius_squared_z: f64,
    pub min_y: i32,
    pub max_y: i32,
}

impl EllipseMask {
    pub fn circle(center: [f64; 2], radius: f64, min_y: i32, max_y: i32) -> Self {
        Self::new(center, radius, radius, min_y, max_y)
    }

    pub fn new(center: [f64; 2], radius_x: f64, radius_z: f64, min_y: i32, max_y: i32) -> Self {
        Self { center, radius_squared_x: radius_x * radius_x, radius_squared_z: radius_z * radius_z, min_y, max_y }
    }

    pub fn test(&self, x: i32, y: i32, z: i32) -> bool {
        self.min_y <= y && self.max_y >= y && self.test_point(x as f64, z as f64)
    }

    pub fn test_point(&self, x: f64, z: f64) -> bool {
        let x = x - self.center[0];
        let z = z - self.center[1];
        (x * x) / self.radius_squared_x + (z * z) / self.radius_squared_z <= 1.0
    }

    pub fn test_area(&self, min_x: i32, min_y: i32, min_z: i32, max_x: i32, max_y: i32, max_z: i32) -> Tristate {
        test_y(min_y, max_y, self.min_y, self.max_y).and(|| self.test_xz(min_x, min_z, max_x, max_z))
    }

    pub fn test_xz(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> Tristate {
        let (x0, z0, x1, z1) = (min_x as f64, min_z as f64, max_x as f64, max_z as f64);
        if self.test_point(x0, z0) && self.test_point(x1, z0) && self.test_point(x0, z1) && self.test_point(x1, z1) {
            return Tristate::True;
        }
        let closest_x = java_clamp(self.center[0], x0, x1);
        let closest_z = java_clamp(self.center[1], z0, z1);
        if !self.test_point(closest_x, closest_z) {
            return Tristate::False;
        }
        Tristate::Undefined
    }

    pub fn is_edge(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> bool {
        self.test_xz(min_x, min_z, max_x, max_z) == Tristate::Undefined
    }
}

/// `Math.clamp(double, double, double)` for `min <= max`: a NaN value stays NaN.
fn java_clamp(value: f64, min: f64, max: f64) -> f64 {
    if value.is_nan() { value } else { value.max(min).min(max) }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PolygonMask {
    /// x/z vertices; `Arc` so per-area submasks stay cheap to clone.
    pub points: Arc<[[f64; 2]]>,
    pub min_y: i32,
    pub max_y: i32,
}

impl PolygonMask {
    pub fn new(points: impl Into<Arc<[[f64; 2]]>>, min_y: i32, max_y: i32) -> Self {
        Self { points: points.into(), min_y, max_y }
    }

    pub fn test(&self, x: i32, y: i32, z: i32) -> bool {
        self.min_y <= y && self.max_y >= y && self.test_column(x, z)
    }

    /// Even-odd ray cast (`testXZ(int, int)`).
    pub fn test_column(&self, x: i32, z: i32) -> bool {
        let (x, z) = (x as f64, z as f64);
        let mut contains = false;
        for ([x1, z1], [x2, z2]) in self.edges() {
            if ((z1 > z) != (z2 > z)) && (x < (x2 - x1) * (z - z1) / (z2 - z1) + x1) {
                contains = !contains;
            }
        }
        contains
    }

    pub fn test_area(&self, min_x: i32, min_y: i32, min_z: i32, max_x: i32, max_y: i32, max_z: i32) -> Tristate {
        test_y(min_y, max_y, self.min_y, self.max_y).and(|| self.test_xz(min_x, min_z, max_x, max_z))
    }

    pub fn test_xz(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> Tristate {
        let (x0, z0, x1, z1) = (min_x as f64, min_z as f64, max_x as f64, max_z as f64);
        let sides = [[x0, z0, x0, z1], [x0, z1, x1, z1], [x1, z1, x1, z0], [x1, z0, x0, z0]];
        for (a, b) in self.edges() {
            if sides.iter().any(|&s| lines_collide(s, [a[0], a[1], b[0], b[1]])) {
                return Tristate::Undefined;
            }
        }
        Tristate::from_bool(self.test_column(min_x, min_z))
    }

    pub fn is_edge(&self, min_x: i32, min_z: i32, max_x: i32, max_z: i32) -> bool {
        self.test_xz(min_x, min_z, max_x, max_z) == Tristate::Undefined
    }

    /// (point i, point i-1) pairs, starting with (first, last).
    fn edges(&self) -> impl Iterator<Item = ([f64; 2], [f64; 2])> + '_ {
        let n = self.points.len();
        (0..n).map(move |i| (self.points[i], self.points[(i + n - 1) % n]))
    }
}

/// Segment intersection; parallel segments divide by zero and yield NaN/inf, which never collide (as in Java).
fn lines_collide([xa1, ya1, xa2, ya2]: [f64; 4], [xb1, yb1, xb2, yb2]: [f64; 4]) -> bool {
    let v = (yb2 - yb1) * (xa2 - xa1) - (xb2 - xb1) * (ya2 - ya1);
    let ua = ((xb2 - xb1) * (ya1 - yb1) - (yb2 - yb1) * (xa1 - xb1)) / v;
    let ub = ((xa2 - xa1) * (ya1 - yb1) - (ya2 - ya1) * (xa1 - xb1)) / v;
    (0.0..=1.0).contains(&ua) && (0.0..=1.0).contains(&ub)
}
