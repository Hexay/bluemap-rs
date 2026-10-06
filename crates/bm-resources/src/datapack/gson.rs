//! Gson's lenient `JsonReader` coercions over an already-parsed [`Value`] (docs/02 §8): numeric strings read as
//! numbers, `nextString` stringifies numbers, and fractional values fail an int read.

use bm_math::Color;
use serde_json::Value;

use super::DataError;

fn invalid(field: &str, expected: &'static str) -> DataError {
    DataError::Invalid { field: field.to_owned(), expected }
}

pub(crate) fn double(v: &Value, field: &str) -> Result<f64, DataError> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .ok_or_else(|| invalid(field, "a number"))
}

/// `nextLong`: an integral value in range; `1e3` or `"7"` pass, `1.5` fails.
pub(crate) fn long(v: &Value, field: &str) -> Result<i64, DataError> {
    if let Some(i) = v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())) {
        return Ok(i);
    }
    let d = double(v, field)?;
    let i = d as i64;
    if i as f64 == d { Ok(i) } else { Err(invalid(field, "an integer")) }
}

pub(crate) fn int(v: &Value, field: &str) -> Result<i32, DataError> {
    let l = long(v, field)?;
    i32::try_from(l).map_err(|_| invalid(field, "an int"))
}

/// The `Boolean` type adapter: a JSON boolean, or a string read with `Boolean.parseBoolean`.
pub(crate) fn boolean(v: &Value, field: &str) -> Result<bool, DataError> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::String(s) => Ok(s.eq_ignore_ascii_case("true")),
        _ => Err(invalid(field, "a boolean")),
    }
}

pub(crate) fn string(v: &Value, field: &str) -> Result<String, DataError> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        _ => Err(invalid(field, "a string")),
    }
}

/// `ColorAdapter.read`, straight alpha: `[r,g,b(,a)]` floats, `{r,g,b,a}` (alpha defaults to 1), a
/// [`Color::parse`] string, or an ARGB int whose alpha becomes opaque when zero.
///
/// Upstream's `NULL` case leaves the token unread, so the enclosing object then fails to parse; `null` is an error
/// here for the same outcome.
pub(crate) fn color(v: &Value, field: &str) -> Result<Color, DataError> {
    let mut c = Color::default();
    match v {
        Value::Array(items) => {
            if !(3..=4).contains(&items.len()) {
                return Err(invalid(field, "3 or 4 colour components"));
            }
            let ch = |i: usize| items.get(i).map_or(Ok(1.0), |x| double(x, field).map(|d| d as f32));
            c.set(ch(0)?, ch(1)?, ch(2)?, ch(3)?, false);
        }
        Value::Object(map) => {
            c.a = 1.0;
            for (name, x) in map {
                let x = double(x, field)? as f32;
                match name.as_str() {
                    "r" => c.r = x,
                    "g" => c.g = x,
                    "b" => c.b = x,
                    "a" => c.a = x,
                    _ => {}
                }
            }
        }
        Value::String(s) => {
            c.parse(s).map_err(|_| invalid(field, "a colour"))?;
        }
        Value::Number(_) => {
            let mut argb = int(v, field)?;
            if argb as u32 & 0xFF00_0000 == 0 {
                argb |= 0xFF00_0000u32 as i32;
            }
            c.set_int(argb);
        }
        _ => return Err(invalid(field, "a colour")),
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn rgba(c: Color) -> [f32; 4] {
        [c.r, c.g, c.b, c.a]
    }

    #[test]
    fn color_adapter_forms() {
        let ok = |v: Value| rgba(color(&v, "c").unwrap());
        assert_eq!(ok(json!(4159204)), rgba(*Color::default().set_int(0xFF3F76E4u32 as i32)));
        assert_eq!(ok(json!(0x803F76E4u32 as i32)), rgba(*Color::default().set_int(0x803F76E4u32 as i32)));
        assert_eq!(ok(json!("#3f76e4")), ok(json!(4159204)));
        assert_eq!(ok(json!("4159204")), ok(json!(4159204)));
        assert_eq!(ok(json!([0.5, "0.25", 1])), [0.5, 0.25, 1.0, 1.0]);
        assert_eq!(ok(json!([0.5, 0.25, 1, 0.5])), [0.5, 0.25, 1.0, 0.5]);
        assert_eq!(ok(json!({"g": 0.5, "x": 3})), [0.0, 0.5, 0.0, 1.0]);
        assert!(!color(&json!([1, 2, 3, 4]), "c").unwrap().premultiplied);
        for bad in [
            json!(null),
            json!(true),
            json!([1, 2]),
            json!([1, 2, 3, 4, 5]),
            json!([1, null, 3]),
            json!("nope"),
            json!(1.5),
        ] {
            assert!(color(&bad, "c").is_err(), "{bad}");
        }
        assert!(color(&json!(4294967295i64), "c").is_err());
    }

    #[test]
    fn lenient_scalars() {
        assert_eq!(int(&json!(1e2), "i").unwrap(), 100);
        assert_eq!(int(&json!("-64"), "i").unwrap(), -64);
        assert!(int(&json!(1.5), "i").is_err());
        assert!(int(&json!(3_000_000_000i64), "i").is_err());
        assert_eq!(long(&json!(6000), "l").unwrap(), 6000);
        assert_eq!(double(&json!("0.25"), "d").unwrap(), 0.25);
        assert!(double(&json!(true), "d").is_err());
        assert!(boolean(&json!("TRUE"), "b").unwrap());
        assert!(!boolean(&json!("yes"), "b").unwrap());
        assert!(boolean(&json!(1), "b").is_err());
        assert_eq!(string(&json!(5), "s").unwrap(), "5");
    }
}
