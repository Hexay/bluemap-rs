//! Substitution resolution and value concatenation over the unresolved document.

use super::raw::{Raw, RawObj, Subst};
use crate::error::ParseError;
use crate::value::{Map, Value};

pub(crate) fn resolve(root: &RawObj, origin: &str) -> Result<Map, ParseError> {
    Resolver { root, origin, stack: Vec::new() }.object(root)
}

struct Resolver<'a> {
    root: &'a RawObj,
    origin: &'a str,
    /// Paths being resolved right now, to report cycles instead of overflowing the stack.
    stack: Vec<Vec<String>>,
}

type RResult<T> = Result<T, ParseError>;

impl<'a> Resolver<'a> {
    fn err(&self, line: usize, col: usize, message: String) -> ParseError {
        ParseError { origin: self.origin.to_owned(), line, col, message }
    }

    fn object(&mut self, obj: &RawObj) -> RResult<Map> {
        let mut map = Map::new();
        for (key, raw) in &obj.fields {
            if let Some(value) = self.value(raw)? {
                map.insert(key.clone(), value);
            }
        }
        Ok(map)
    }

    /// `None` = undefined (an optional substitution that found nothing): the field or list element disappears.
    fn value(&mut self, raw: &Raw) -> RResult<Option<Value>> {
        Ok(match raw {
            Raw::Lit { value, .. } => Some(value.clone()),
            Raw::Ws(s) => Some(Value::String(s.clone())),
            Raw::Obj(obj) => Some(Value::Object(self.object(obj)?)),
            Raw::List(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.extend(self.value(item)?);
                }
                Some(Value::List(out))
            }
            Raw::Subst(s) => self.substitution(s)?,
            Raw::Concat { pieces, line, col } => self.concat(pieces, *line, *col)?,
            Raw::OrElse(value, previous) => match self.value(value)? {
                Some(v) => Some(v),
                None => self.value(previous)?,
            },
        })
    }

    fn substitution(&mut self, s: &Subst) -> RResult<Option<Value>> {
        if !s.prefix.is_empty() {
            let relative: Vec<String> = s.prefix.iter().chain(&s.path).cloned().collect();
            if let Some(v) = self.lookup(&relative, s)? {
                return Ok(Some(v));
            }
        }
        if let Some(v) = self.lookup(&s.path, s)? {
            return Ok(Some(v));
        }
        let joined = s.path.join(".");
        if let Ok(env) = std::env::var(&joined) {
            return Ok(Some(Value::String(env)));
        }
        if s.optional {
            return Ok(None);
        }
        Err(self.err(
            s.line,
            s.col,
            format!("could not resolve substitution ${{{joined}}}: no such setting and no environment variable '{joined}' (use ${{?{joined}}} if it is optional)"),
        ))
    }

    fn lookup(&mut self, path: &[String], at: &Subst) -> RResult<Option<Value>> {
        let mut obj: &'a RawObj = self.root;
        for (i, key) in path.iter().enumerate() {
            let Some(node) = obj.get(key) else { return Ok(None) };
            match node {
                Raw::Obj(inner) if i + 1 < path.len() => obj = inner,
                _ => {
                    let value = self.guarded(&path[..=i], node, at)?;
                    return Ok(value.and_then(|v| v.at(&path[i + 1..]).cloned()));
                }
            }
        }
        Ok(None)
    }

    fn guarded(&mut self, path: &[String], node: &Raw, at: &Subst) -> RResult<Option<Value>> {
        if self.stack.iter().any(|p| p == path) {
            let msg = format!("substitution cycle: ${{{}}} depends on itself", path.join("."));
            return Err(self.err(at.line, at.col, msg));
        }
        self.stack.push(path.to_vec());
        let result = self.value(node);
        self.stack.pop();
        result
    }

    fn concat(&mut self, pieces: &[Raw], line: usize, col: usize) -> RResult<Option<Value>> {
        // (value, text inside a string concatenation, is whitespace)
        let mut parts: Vec<(Value, Option<String>, bool)> = Vec::new();
        for piece in pieces {
            match piece {
                Raw::Ws(s) => parts.push((Value::String(s.clone()), Some(s.clone()), true)),
                Raw::Lit { value, text } => parts.push((value.clone(), Some(text.clone()), false)),
                other => parts.extend(self.value(other)?.map(|v| (v, None, false))),
            }
        }
        let solid: Vec<&Value> = parts.iter().filter(|(_, _, ws)| !ws).map(|(v, _, _)| v).collect();
        if solid.is_empty() {
            return Ok(None);
        }
        if parts.len() == 1 {
            return Ok(parts.pop().map(|(v, _, _)| v));
        }
        if solid.iter().any(|v| matches!(v, Value::List(_) | Value::Object(_))) {
            return self.concat_containers(solid, line, col).map(Some);
        }
        let text = parts.into_iter().map(|(v, text, _)| text.unwrap_or_else(|| concat_text(&v))).collect();
        Ok(Some(Value::String(text)))
    }

    fn concat_containers(&self, solid: Vec<&Value>, line: usize, col: usize) -> RResult<Value> {
        if solid.iter().all(|v| matches!(v, Value::List(_))) {
            let items = solid.into_iter().flat_map(|v| match v {
                Value::List(items) => items.clone(),
                _ => unreachable!(),
            });
            return Ok(Value::List(items.collect()));
        }
        if solid.iter().all(|v| matches!(v, Value::Object(_))) {
            let mut merged = Map::new();
            for v in solid {
                if let Value::Object(map) = v {
                    merged.merge(map.clone());
                }
            }
            return Ok(Value::Object(merged));
        }
        let kinds: Vec<&str> = solid.iter().map(|v| v.type_name()).collect();
        Err(self.err(
            line,
            col,
            format!(
                "cannot concatenate {} (lists only join lists, objects only merge with objects; quote text values)",
                kinds.join(" with ")
            ),
        ))
    }
}

fn concat_text(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.clone(),
        Value::List(_) | Value::Object(_) => unreachable!("containers are concatenated separately"),
    }
}
