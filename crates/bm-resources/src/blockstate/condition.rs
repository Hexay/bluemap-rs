//! `BlockStateCondition`: property matching for variant keys and multipart `when` clauses.

use bm_world::BlockState;

/// Keys and values are lowercased at construction; state properties are compared as-is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    All,
    None,
    Property(Box<str>, Box<str>),
    PropertySet(Box<str>, Box<[Box<str>]>),
    And(Box<[Condition]>),
    Or(Box<[Condition]>),
}

impl Condition {
    pub fn matches(&self, state: &BlockState) -> bool {
        match self {
            Self::All => true,
            Self::None => false,
            Self::Property(key, value) => state.property(key) == Some(value),
            Self::PropertySet(key, values) => state.property(key).is_some_and(|v| values.iter().any(|x| **x == *v)),
            Self::And(conditions) => conditions.iter().all(|c| c.matches(state)),
            Self::Or(conditions) => conditions.iter().any(|c| c.matches(state)),
        }
    }

    pub fn property(key: &str, value: &str) -> Self {
        Self::Property(key.to_lowercase().into(), value.to_lowercase().into())
    }

    /// `property(key, values...)`; `None` for an empty list (Java's precondition throws).
    pub fn property_set(key: &str, values: &[&str]) -> Option<Self> {
        match values {
            [] => None,
            [value] => Some(Self::property(key, value)),
            _ => Some(Self::PropertySet(
                key.to_lowercase().into(),
                values.iter().map(|v| v.to_lowercase().into()).collect(),
            )),
        }
    }

    /// `and(...)`: a single condition is returned unwrapped; `None` for an empty list.
    pub fn and(mut conditions: Vec<Self>) -> Option<Self> {
        match conditions.len() {
            0 => None,
            1 => conditions.pop(),
            _ => Some(Self::And(conditions.into())),
        }
    }

    /// `or(...)`: a single condition is returned unwrapped; `None` for an empty list.
    pub fn or(mut conditions: Vec<Self>) -> Option<Self> {
        match conditions.len() {
            0 => None,
            1 => conditions.pop(),
            _ => Some(Self::Or(conditions.into())),
        }
    }

    /// A `variants` key: `""`/`default`/`normal` → [`All`](Self::All), `a=b,c=d` → AND of properties (no `|`
    /// alternatives here). Elements without `=` are skipped; if nothing else parsed the key is [`None`](Self::None).
    pub fn parse_variant_key(key: &str) -> Self {
        if key.is_empty() || key == "default" || key == "normal" {
            return Self::All;
        }
        let mut conditions = Vec::new();
        let mut invalid = false;
        for element in java_split(key, ',') {
            match element.split_once('=') {
                Some((k, v)) => conditions.push(Self::property(k, v)),
                None => invalid = true,
            }
        }
        match Self::and(conditions) {
            Some(c) => c,
            None if invalid => Self::None,
            // "," splits to nothing in Java, so it reads as the default variant
            None => Self::All,
        }
    }
}

/// Java's `String.split` for a literal separator: trailing empty pieces are dropped, but a string without the
/// separator (including `""`) is returned whole.
pub(crate) fn java_split(s: &str, sep: char) -> Vec<&str> {
    if !s.contains(sep) {
        return vec![s];
    }
    let mut parts: Vec<&str> = s.split(sep).collect();
    while parts.last() == Some(&"") {
        parts.pop();
    }
    parts
}
