//! Unresolved document tree: parsed values that may still contain substitutions and concatenations.

use crate::value::Value;

#[derive(Debug, Clone)]
pub(crate) struct Subst {
    pub path: Vec<String>,
    pub optional: bool,
    /// Location of the include that brought this substitution in; looked up relative to it first.
    pub prefix: Vec<String>,
    pub line: usize,
    pub col: usize,
}

impl Subst {
    fn refers_to(&self, abs: &[String]) -> bool {
        self.path == abs || (!self.prefix.is_empty() && self.prefix.iter().chain(&self.path).eq(abs.iter()))
    }
}

#[derive(Debug, Clone)]
pub(crate) enum Raw {
    /// Scalar literal; `text` is how it reads inside a string concatenation (numbers keep their source text).
    Lit {
        value: Value,
        text: String,
    },
    /// Whitespace between concatenated values.
    Ws(String),
    Obj(RawObj),
    List(Vec<Raw>),
    Concat {
        pieces: Vec<Raw>,
        line: usize,
        col: usize,
    },
    Subst(Subst),
    /// A reassignment that may resolve to nothing (`a: ${?x}`): then the earlier value stays.
    OrElse(Box<Raw>, Box<Raw>),
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RawObj {
    pub fields: Vec<(String, Raw)>,
}

impl RawObj {
    pub fn get(&self, key: &str) -> Option<&Raw> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    fn slot(&mut self, key: &str) -> Option<&mut Raw> {
        self.fields.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// The object at `path`, created on the way; an existing non-object is replaced (HOCON: objects don't merge
    /// with scalars).
    pub fn object_at(&mut self, path: &[String]) -> &mut RawObj {
        let Some((key, rest)) = path.split_first() else { return self };
        if !matches!(self.get(key), Some(Raw::Obj(_))) {
            self.put(key, Raw::Obj(RawObj::default()));
        }
        match self.slot(key) {
            Some(Raw::Obj(obj)) => obj.object_at(rest),
            _ => unreachable!("just inserted an object"),
        }
    }

    fn put(&mut self, key: &str, value: Raw) {
        match self.slot(key) {
            Some(slot) => *slot = value,
            None => self.fields.push((key.to_owned(), value)),
        }
    }

    /// Assigns `value` at `path` (relative to this object, whose absolute location is `abs_prefix`). Objects merge
    /// into an existing object; self-references (`a: ${a} [2]`, `a += 2`) see the previous value.
    pub fn set(&mut self, path: &[String], value: Raw, abs_prefix: &[String]) {
        let (key, parents) = path.split_last().expect("paths are never empty");
        let abs: Vec<String> = abs_prefix.iter().chain(path).cloned().collect();
        let target = self.object_at(parents);
        let previous = target.get(key);
        let value = replace_self_refs(value, &abs, previous);
        match (target.slot(key), value) {
            (Some(Raw::Obj(existing)), Raw::Obj(new)) => existing.merge(new),
            (Some(prev), value @ (Raw::Subst(_) | Raw::Concat { .. })) => {
                *prev = Raw::OrElse(Box::new(value), Box::new(prev.clone()));
            }
            (_, value) => target.put(key, value),
        }
    }

    fn merge(&mut self, other: RawObj) {
        for (key, value) in other.fields {
            match (self.slot(&key), value) {
                (Some(Raw::Obj(existing)), Raw::Obj(new)) => existing.merge(new),
                (_, value) => self.put(&key, value),
            }
        }
    }
}

/// Replaces substitutions of `abs` itself with the value it had before this assignment (dropped when there was
/// none and the substitution is optional).
fn replace_self_refs(value: Raw, abs: &[String], previous: Option<&Raw>) -> Raw {
    let swap = |raw: Raw| -> Option<Raw> {
        match raw {
            Raw::Subst(s) if s.refers_to(abs) => match previous {
                Some(prev) => Some(prev.clone()),
                None if s.optional => None,
                None => Some(Raw::Subst(s)),
            },
            subst @ Raw::Subst(_) => Some(subst),
            other => Some(replace_self_refs(other, abs, previous)),
        }
    };
    match value {
        Raw::Subst(_) => swap(value).unwrap_or(Raw::Concat { pieces: vec![], line: 0, col: 0 }),
        Raw::Concat { pieces, line, col } => {
            Raw::Concat { pieces: pieces.into_iter().filter_map(swap).collect(), line, col }
        }
        Raw::List(items) => Raw::List(items.into_iter().filter_map(swap).collect()),
        other => other,
    }
}
