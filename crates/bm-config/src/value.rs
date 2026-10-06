//! Resolved config tree, shaped like the Configurate node tree BlueMap's config classes are mapped from.

/// A resolved HOCON value. Object members that are `null` are kept here but treated as absent by the typed layer,
/// exactly like Configurate (which drops them on load).
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// typesafe-config `ConfigInt`/`ConfigLong`.
    Int(i64),
    /// typesafe-config `ConfigDouble`.
    Float(f64),
    String(String),
    List(Vec<Value>),
    Object(Map),
}

/// Insertion-ordered object. Duplicate keys replace in place.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Map {
    entries: Vec<(String, Value)>,
}

pub(crate) static EMPTY_MAP: Map = Map { entries: Vec::new() };

impl Map {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn insert(&mut self, key: String, value: Value) {
        match self.entries.iter_mut().find(|(k, _)| *k == key) {
            Some((_, slot)) => *slot = value,
            None => self.entries.push((key, value)),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Deep merge: objects merge recursively, anything else in `other` replaces.
    pub fn merge(&mut self, other: Map) {
        for (key, value) in other.entries {
            match (self.entries.iter_mut().find(|(k, _)| *k == key), value) {
                (Some((_, Value::Object(mine))), Value::Object(theirs)) => mine.merge(theirs),
                (_, value) => self.insert(key, value),
            }
        }
    }
}

impl Value {
    /// Object member lookup; `None` for non-objects (Configurate: a virtual node).
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn at<S: AsRef<str>>(&self, path: &[S]) -> Option<&Value> {
        path.iter().try_fold(self, |v, key| v.get(key.as_ref()))
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Map> {
        match self {
            Value::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "a boolean",
            Value::Int(_) | Value::Float(_) => "a number",
            Value::String(_) => "a string",
            Value::List(_) => "a list",
            Value::Object(_) => "an object",
        }
    }

    /// JSON view of the node tree as Configurate's Gson loader writes it (null object members dropped). This is
    /// how BlueMap hands `marker-sets` to its marker parser.
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::Value as J;
        match self {
            Value::Null => J::Null,
            Value::Bool(b) => J::Bool(*b),
            Value::Int(i) => J::from(*i),
            Value::Float(f) => serde_json::Number::from_f64(*f).map_or(J::Null, J::Number),
            Value::String(s) => J::String(s.clone()),
            Value::List(items) => J::Array(items.iter().map(Value::to_json).collect()),
            Value::Object(map) => J::Object(
                map.iter()
                    .filter(|(_, v)| !matches!(v, Value::Null))
                    .map(|(k, v)| (k.to_owned(), v.to_json()))
                    .collect(),
            ),
        }
    }
}

/// Java's `Double.toString` (shortest repr, JDK 19+): plain notation in [1e-3, 1e7), else `d.dddE±n`.
pub fn java_double_to_string(d: f64) -> String {
    if d.is_nan() {
        return "NaN".into();
    }
    if d.is_infinite() {
        return if d > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if d == 0.0 || (1e-3..1e7).contains(&d.abs()) {
        let s = format!("{d}");
        return if s.contains('.') { s } else { s + ".0" };
    }
    let sci = format!("{d:e}");
    let (mantissa, exp) = sci.split_once('e').expect("{:e} always has an exponent");
    let mantissa = if mantissa.contains('.') { mantissa.to_owned() } else { format!("{mantissa}.0") };
    format!("{mantissa}E{exp}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_double_strings() {
        for (d, s) in [
            (1.0, "1.0"),
            (1.5, "1.5"),
            (-0.0, "-0.0"),
            (100.0, "100.0"),
            (0.001, "0.001"),
            (0.0001, "1.0E-4"),
            (1e7, "1.0E7"),
            (1.234e10, "1.234E10"),
            (9999999.0, "9999999.0"),
            (f64::INFINITY, "Infinity"),
        ] {
            assert_eq!(java_double_to_string(d), s);
        }
    }

    #[test]
    fn merge_is_deep() {
        let mut a = Map::new();
        let mut inner = Map::new();
        inner.insert("x".into(), Value::Int(1));
        a.insert("o".into(), Value::Object(inner.clone()));
        let mut b = Map::new();
        inner = Map::new();
        inner.insert("y".into(), Value::Int(2));
        b.insert("o".into(), Value::Object(inner));
        a.merge(b);
        let o = a.get("o").unwrap();
        assert_eq!(o.get("x"), Some(&Value::Int(1)));
        assert_eq!(o.get("y"), Some(&Value::Int(2)));
    }
}
