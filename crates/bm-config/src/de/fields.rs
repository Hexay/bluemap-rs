//! `deserialize_with` helpers for BlueMap's custom Configurate type serializers.

use serde::de::{Deserialize, Deserializer, Error};

use super::coerce;
use crate::key::Key;
use crate::value::Value;

/// `KeyTypeSerializer`: `node.getString()` is null for lists/objects, giving no key.
pub(crate) fn opt_key<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Key>, D::Error> {
    let v = Value::deserialize(d)?;
    Ok(coerce::to_string(&v).ok().map(|s| Key::parse(&s)))
}

/// `RegistryTypeSerializer<WorldLoaderType>`: only `bluemap:anvil` exists; non-scalars fall back to it.
pub(crate) fn world_loader<'de, D: Deserializer<'de>>(d: D) -> Result<Key, D::Error> {
    let v = Value::deserialize(d)?;
    let Ok(s) = coerce::to_string(&v) else { return Ok(Key::bluemap("anvil")) };
    let key = Key::parse_with_default(&s, Key::BLUEMAP);
    match key == Key::bluemap("anvil") {
        true => Ok(key),
        false => Err(D::Error::custom(format!("unknown world loader '{s}' (the only one is bluemap:anvil)"))),
    }
}

fn vector<'de, D: Deserializer<'de>, T: Default>(
    d: D,
    coerce_one: impl Fn(&Value) -> Option<T>,
) -> Result<[T; 2], D::Error> {
    let v = Value::deserialize(d)?;
    let present = |k: &str| v.get(k).filter(|c| !matches!(c, Value::Null));
    let (Some(x), Some(y)) = (present("x"), present("y").or_else(|| present("z"))) else {
        return Err(D::Error::custom("expected { x: ..., z: ... } (value x or y/z missing)"));
    };
    // Configurate's node.getInt()/getDouble() return 0 for values that don't coerce
    Ok([coerce_one(x).unwrap_or_default(), coerce_one(y).unwrap_or_default()])
}

/// `Vector2iTypeSerializer`.
pub(crate) fn vec2i<'de, D: Deserializer<'de>>(d: D) -> Result<[i32; 2], D::Error> {
    vector(d, |v| coerce::to_int(v, i32::MIN as i64, i32::MAX as i64, "an integer").ok().map(|i| i as i32))
}

/// `Vector2dTypeSerializer`.
pub(crate) fn vec2d<'de, D: Deserializer<'de>>(d: D) -> Result<[f64; 2], D::Error> {
    vector(d, |v| coerce::to_f64(v).ok())
}

/// `Vector2d[]` (polygon shapes).
pub(crate) fn vec2d_list<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<[f64; 2]>>, D::Error> {
    #[derive(serde::Deserialize)]
    struct V(#[serde(deserialize_with = "vec2d")] [f64; 2]);
    let items: Vec<V> = Vec::deserialize(d)?;
    Ok(Some(items.into_iter().map(|V(p)| p).collect()))
}

/// `LinkedHashSet<String>`: order kept, duplicates dropped.
pub(crate) fn string_set<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let mut out: Vec<String> = Vec::new();
    for s in Vec::<String>::deserialize(d)? {
        if !out.contains(&s) {
            out.push(s);
        }
    }
    Ok(out)
}

/// `Map<String, String>`: members in file order; a non-object gives an empty map.
pub(crate) fn string_map<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(String, String)>, D::Error> {
    let Value::Object(map) = Value::deserialize(d)? else { return Ok(vec![]) };
    map.iter()
        .filter(|(_, v)| !matches!(v, Value::Null))
        .map(|(k, v)| {
            coerce::to_string(v).map(|s| (k.to_owned(), s)).map_err(|e| D::Error::custom(format!("{k}: {e}")))
        })
        .collect()
}
