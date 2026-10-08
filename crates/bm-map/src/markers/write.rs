//! `MarkerGson.INSTANCE.toJson`: reflective field order (subclass fields first, then each superclass's), doubles
//! and floats via Java's `toString`, vectors rounded to 4 decimals and written as longs when whole.

use bm_java::fmt::{double_to_string, float_to_string};

use super::model::{Color, Head, Marker};
use crate::gson::{JsonObject, array, string_array};

impl Marker {
    pub fn to_json(&self) -> String {
        let mut json = match &self.head {
            Head::Poi { classes, detail, icon, anchor } => JsonObject::new()
                .raw("classes", &string_array(classes))
                .string("detail", detail)
                .string("icon", icon)
                .raw("anchor", &vec2i(*anchor)),
            Head::Html { classes, anchor, html } => JsonObject::new()
                .raw("classes", &string_array(classes))
                .raw("anchor", &vec2i(*anchor))
                .string("html", html),
            Head::Line { line } => JsonObject::new().raw("line", &array(line.iter().map(|p| vec3(*p)))),
            Head::Shape { shape, holes, y } => shape_fields(shape, holes).raw("shapeY", &float_to_string(*y)),
            Head::Extrude { shape, holes, min_y, max_y } => shape_fields(shape, holes)
                .raw("shapeMinY", &float_to_string(*min_y))
                .raw("shapeMaxY", &float_to_string(*max_y)),
        };
        if let Some(style) = &self.style {
            json = json
                .bool("depthTest", style.depth_test)
                .int("lineWidth", style.line_width)
                .raw("lineColor", &color(style.line_color));
            if let Some(fill) = style.fill_color {
                json = json.raw("fillColor", &color(fill));
            }
        }
        if let Some(object) = &self.object {
            json = json
                .string("detail", &object.detail)
                .opt_string("link", object.link.as_deref())
                .bool("newTab", object.new_tab);
        }
        json.raw("minDistance", &double_to_string(self.min_distance))
            .raw("maxDistance", &double_to_string(self.max_distance))
            .string("type", &self.kind)
            .string("label", &self.label)
            .raw("position", &vec3(self.position))
            .int("sorting", self.sorting)
            .bool("listed", self.listed)
            .finish()
    }
}

fn shape_fields(shape: &[[f64; 2]], holes: &[Vec<[f64; 2]>]) -> JsonObject {
    let shape_json = |points: &[[f64; 2]]| array(points.iter().map(|&[x, z]| xyz(&[("x", x), ("z", z)])));
    JsonObject::new().raw("shape", &shape_json(shape)).raw("holes", &array(holes.iter().map(|h| shape_json(h))))
}

fn vec3([x, y, z]: [f64; 3]) -> String {
    xyz(&[("x", x), ("y", y), ("z", z)])
}

fn xyz(components: &[(&str, f64)]) -> String {
    components.iter().fold(JsonObject::new(), |json, &(name, v)| json.raw(name, &rounded(v))).finish()
}

fn vec2i([x, y]: [i32; 2]) -> String {
    JsonObject::new().int("x", x).int("y", y).finish()
}

/// `ColorAdapter.write`: the float alpha goes through `JsonWriter.value(double)`.
fn color(c: Color) -> String {
    JsonObject::new().int("r", c.r).int("g", c.g).int("b", c.b).raw("a", &double_to_string(f64::from(c.a))).finish()
}

/// `writeRounded`: `Math.round(v * 10000d) / 10000d`, as a long when whole.
pub(super) fn rounded(v: f64) -> String {
    let d = java_round(v * 10000.0) as f64 / 10000.0;
    if d == (d as i64) as f64 { (d as i64).to_string() } else { double_to_string(d) }
}

/// `Math.round(double)`: floor(v + 1/2) computed exactly, saturating like Java's `(long)` cast.
fn java_round(v: f64) -> i64 {
    let floor = v.floor();
    // the fraction v - floor is exact for every finite double
    (if v - floor >= 0.5 { floor + 1.0 } else { floor }) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_matches_java() {
        // Java 5.28 output for these config values (tests/data/markers)
        for (v, want) in [
            (1.23456789, "1.2346"),
            (-0.00005, "0"),
            (0.00004, "0"),
            (1e-5, "0"),
            (1e15, "9.223372036854776E14"),
            (123456.78901, "123456.789"),
            (1e-4, "1.0E-4"),
            (0.33333333, "0.3333"),
            (-100.5, "-100.5"),
            (64.0, "64"),
        ] {
            assert_eq!(rounded(v), want, "{v}");
        }
        assert_eq!(java_round(-2.5), -2);
        assert_eq!(java_round(0.49999999999999994), 0);
    }
}
