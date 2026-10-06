//! Blockstate JSON → [`BlockStateDef`], following BlueMap's Gson adapters (`Variants.Adapter`,
//! `Multipart.Adapter`, `VariantSet.Adapter`, reflective `Variant`) including where they throw.

use serde_json::{Map, Value};

use super::condition::java_split;
use super::{BlockStateDef, Condition, Multipart, Variant, VariantSet, Variants};
use crate::ResourcePath;

const JSON_COMMENT: &str = "__comment";

/// Why a blockstate file failed to load; BlueMap drops the whole file in each case.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("{field}: expected {expected}, found {found}")]
    Type { field: &'static str, expected: &'static str, found: &'static str },
    #[error("{field}: not a number: {value:?}")]
    Number { field: &'static str, value: String },
    #[error("empty {0} (BlueMap's condition builders reject it)")]
    Empty(&'static str),
}

pub(super) fn block_state(v: &Value) -> Result<BlockStateDef, ParseError> {
    let obj = object(v, "blockstate")?;
    let variants = obj.get("variants").filter(|v| !v.is_null()).map(variants).transpose()?;
    let multipart = obj.get("multipart").filter(|v| !v.is_null()).map(multipart).transpose()?;
    Ok(BlockStateDef { variants, multipart })
}

fn variants(v: &Value) -> Result<Variants, ParseError> {
    let mut sets = Vec::new();
    let mut default = None;
    for (key, value) in object(v, "variants")? {
        if key == JSON_COMMENT {
            continue;
        }
        let condition = Condition::parse_variant_key(key);
        // the set is read before the key is judged, so a broken set under a dropped key still fails the file
        let set = variant_set(value, condition)?;
        match set.condition {
            Condition::All => default = Some(set),
            Condition::None => {}
            _ => sets.push(set),
        }
    }
    Ok(Variants { sets, default })
}

fn multipart(v: &Value) -> Result<Multipart, ParseError> {
    let mut parts = Vec::new();
    for part in array(v, "multipart")? {
        let part = object(part, "multipart part")?;
        let condition = part.get("when").map(when).transpose()?;
        let Some(apply) = part.get("apply").filter(|v| !v.is_null()) else { continue };
        parts.push(variant_set(apply, condition.unwrap_or(Condition::All))?);
    }
    Ok(Multipart { parts })
}

fn when(v: &Value) -> Result<Condition, ParseError> {
    let mut conditions = Vec::new();
    for (key, value) in object(v, "when")? {
        match key.as_str() {
            JSON_COMMENT => {}
            "OR" => conditions.push(Condition::or(when_list(value)?).ok_or(ParseError::Empty("OR"))?),
            "AND" => conditions.push(Condition::and(when_list(value)?).ok_or(ParseError::Empty("AND"))?),
            _ => {
                let joined = match value {
                    Value::Bool(b) => b.to_string(),
                    other => string(other, "when value")?,
                };
                let values = java_split(&joined, '|');
                conditions.push(Condition::property_set(key, &values).ok_or(ParseError::Empty("when value"))?);
            }
        }
    }
    Condition::and(conditions).ok_or(ParseError::Empty("when"))
}

fn when_list(v: &Value) -> Result<Vec<Condition>, ParseError> {
    array(v, "OR/AND")?.iter().map(when).collect()
}

fn variant_set(v: &Value, condition: Condition) -> Result<VariantSet, ParseError> {
    let variants: Box<[Variant]> = match v {
        Value::Array(items) => items.iter().map(variant).collect::<Result<_, _>>()?,
        other => Box::new([variant(other)?]),
    };
    Ok(VariantSet::new(condition, variants))
}

fn variant(v: &Value) -> Result<Variant, ParseError> {
    let obj = object(v, "variant")?;
    let field = |name: &str| obj.get(name).filter(|v| !v.is_null());
    let renderer = match obj.get("renderer") {
        // RegistryAdapter is not null-safe: nextString() throws on null
        Some(v) => ResourcePath::key_in(&string(v, "renderer")?, "bluemap"),
        None => ResourcePath::key(Variant::DEFAULT_RENDERER),
    };
    // Java would store a null model and fail at render time; treat it as absent instead
    let model = field("model").map(|v| string(v, "model")).transpose()?;
    let model = model.map_or_else(|| ResourcePath::key(Variant::MISSING_MODEL), |m| ResourcePath::parse(&m));
    let rotation =
        |name: &'static str| field(name).map(|v| number(v, name)).transpose().map(|n| n.unwrap_or(0.0) as f32);
    let uvlock = field("uvlock").map(boolean).transpose()?.unwrap_or(false);
    let weight = field("weight").map(|v| number(v, "weight")).transpose()?.unwrap_or(1.0);
    Ok(Variant::with_renderer(renderer, model, rotation("x")?, rotation("y")?, rotation("z")?, uvlock, weight))
}

fn object<'a>(v: &'a Value, field: &'static str) -> Result<&'a Map<String, Value>, ParseError> {
    v.as_object().ok_or_else(|| mismatch(field, "an object", v))
}

fn array<'a>(v: &'a Value, field: &'static str) -> Result<&'a [Value], ParseError> {
    v.as_array().map(Vec::as_slice).ok_or_else(|| mismatch(field, "an array", v))
}

/// Gson `nextString`: strings, and numbers as their text.
fn string(v: &Value, field: &'static str) -> Result<String, ParseError> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(n) => Ok(n.to_string()),
        other => Err(mismatch(field, "a string", other)),
    }
}

/// Gson `nextDouble`: numbers, and strings through `Double.parseDouble`.
fn number(v: &Value, field: &'static str) -> Result<f64, ParseError> {
    match v {
        Value::Number(n) => Ok(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => parse_java_double(s).ok_or_else(|| ParseError::Number { field, value: s.clone() }),
        other => Err(mismatch(field, "a number", other)),
    }
}

/// Gson's boolean adapter: `true`/`false`, or a string through `Boolean.parseBoolean`.
fn boolean(v: &Value) -> Result<bool, ParseError> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::String(s) => Ok(s.eq_ignore_ascii_case("true")),
        other => Err(mismatch("uvlock", "a boolean", other)),
    }
}

fn parse_java_double(s: &str) -> Option<f64> {
    let s = s.trim();
    let s = s.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(s);
    s.parse().ok()
}

fn mismatch(field: &'static str, expected: &'static str, found: &Value) -> ParseError {
    let found = match found {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    };
    ParseError::Type { field, expected, found }
}
