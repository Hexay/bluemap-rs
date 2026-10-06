//! Field readers with Gson's coercions (docs/02 §8). Primitive fields keep their default on `null`; the
//! registered vector/direction/axis adapters are not null-safe, so `null` there fails the file, as upstream.

use serde_json::{Map, Value};

use super::ModelError;

pub type Obj = Map<String, Value>;

fn type_err<T>(field: &'static str, expected: &'static str) -> Result<T, ModelError> {
    Err(ModelError::Type { field, expected })
}

pub fn object<'a>(v: &'a Value, field: &'static str) -> Result<&'a Obj, ModelError> {
    v.as_object().map_or_else(|| type_err(field, "an object"), Ok)
}

/// The field's value, treating `null` like an absent field (null-safe adapters).
pub fn non_null<'a>(obj: &'a Obj, key: &str) -> Option<&'a Value> {
    obj.get(key).filter(|v| !v.is_null())
}

/// `Double.parseDouble`, as Gson's `nextDouble`/`nextInt` apply it to quoted numbers.
fn parse_java_double(s: &str) -> Option<f64> {
    let t = s.trim();
    let t = t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t);
    match t {
        "NaN" | "+NaN" | "-NaN" => Some(f64::NAN),
        "Infinity" | "+Infinity" => Some(f64::INFINITY),
        "-Infinity" => Some(f64::NEG_INFINITY),
        _ if t.bytes().any(|c| c.is_ascii_alphabetic() && !matches!(c, b'e' | b'E')) => None,
        _ => t.parse().ok(),
    }
}

/// `nextDouble` cast to float, as the `Vector*fAdapter`s and float fields read numbers.
pub fn f32_of(v: &Value, field: &'static str) -> Result<f32, ModelError> {
    let d = match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => parse_java_double(s),
        _ => None,
    };
    d.map_or_else(|| type_err(field, "a number"), |d| Ok(d as f32))
}

/// `nextInt`: integral values only, also from quoted numbers.
pub fn i32_of(v: &Value, field: &'static str) -> Result<i32, ModelError> {
    let d = match v {
        Value::Number(n) => match n.as_i64() {
            Some(i) => return i32::try_from(i).map_or_else(|_| type_err(field, "an int"), Ok),
            None => n.as_f64(),
        },
        Value::String(s) => match s.parse::<i32>() {
            Ok(i) => return Ok(i),
            Err(_) => parse_java_double(s),
        },
        _ => None,
    };
    match d {
        Some(d) if (d as i32) as f64 == d => Ok(d as i32),
        _ => type_err(field, "an int"),
    }
}

/// `nextString`: strings and numbers.
pub fn string_of(v: &Value, field: &'static str) -> Result<String, ModelError> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        _ => type_err(field, "a string"),
    }
}

pub fn opt_f32(obj: &Obj, key: &'static str, default: f32) -> Result<f32, ModelError> {
    non_null(obj, key).map_or(Ok(default), |v| f32_of(v, key))
}

pub fn opt_i32(obj: &Obj, key: &'static str, default: i32) -> Result<i32, ModelError> {
    non_null(obj, key).map_or(Ok(default), |v| i32_of(v, key))
}

/// Gson's boolean adapter: a quoted value is `Boolean.parseBoolean` (anything but "true" is false).
pub fn opt_bool(obj: &Obj, key: &'static str) -> Result<Option<bool>, ModelError> {
    match non_null(obj, key) {
        None => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(Value::String(s)) => Ok(Some(s.eq_ignore_ascii_case("true"))),
        Some(_) => type_err(key, "a boolean"),
    }
}

/// A `Vector3f`/`Vector4f` array of exactly `N` numbers; `null` is an error.
pub fn opt_vec<const N: usize>(obj: &Obj, key: &'static str) -> Result<Option<[f32; N]>, ModelError> {
    let Some(v) = obj.get(key) else { return Ok(None) };
    let items = match v.as_array() {
        Some(items) if items.len() == N => items,
        _ => return type_err(key, "an array of numbers of the right length"),
    };
    let mut out = [0.0; N];
    for (o, item) in out.iter_mut().zip(items) {
        *o = f32_of(item, key)?;
    }
    Ok(Some(out))
}

/// A field read by a non-null-safe string adapter (`Direction`, `Axis`).
pub fn opt_strict_string(obj: &Obj, key: &'static str) -> Result<Option<String>, ModelError> {
    obj.get(key).map(|v| string_of(v, key)).transpose()
}
