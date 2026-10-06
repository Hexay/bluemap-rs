//! JSON as Gson's lenient `JsonReader` accepts it (BlueMap reads every pack file that way, docs/02 §8): `//`, `/* */`
//! and `#` comments, unquoted and single-quoted names and strings, `=`/`=>` after names, `;` as a separator,
//! trailing commas, and empty array slots as null. Parses to `serde_json::Value`; typed reading happens after.

use serde_json::{Map, Number, Value};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid JSON at byte {pos}: {msg}")]
pub struct JsonError {
    pub pos: usize,
    pub msg: &'static str,
}

pub fn parse(src: &str) -> Result<Value, JsonError> {
    let mut p = Parser { s: src.as_bytes(), pos: 0, depth: 0 };
    p.skip_bom();
    let v = p.value()?;
    p.ws()?;
    // Gson stops after the first value; trailing content is ignored rather than rejected
    Ok(v)
}

/// A top-level object's members in file order, duplicates kept, for loaders that stream entries like Gson's
/// `JsonReader` (first-wins configs). On a syntax error the members read before it are returned with the error.
pub fn parse_entries(src: &str) -> (Vec<(String, Value)>, Option<JsonError>) {
    let mut p = Parser { s: src.as_bytes(), pos: 0, depth: 1 };
    p.skip_bom();
    let mut entries = Vec::new();
    let res = p.ws().and_then(|()| match p.peek() {
        Some(b'{') => p.members(|k, v| entries.push((k, v))),
        _ => p.err("expected an object"),
    });
    (entries, res.err())
}

/// [`parse`] then deserialize into `T`.
pub fn from_str<T: serde::de::DeserializeOwned>(src: &str) -> Result<T, crate::Error> {
    Ok(serde_json::from_value(parse(src)?)?)
}

const MAX_DEPTH: u32 = 256;

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
    depth: u32,
}

impl Parser<'_> {
    fn err<T>(&self, msg: &'static str) -> Result<T, JsonError> {
        Err(JsonError { pos: self.pos, msg })
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.pos).copied()
    }

    fn skip_bom(&mut self) {
        if self.s.starts_with("\u{feff}".as_bytes()) {
            self.pos = 3;
        }
    }

    /// Skips whitespace and comments.
    fn ws(&mut self) -> Result<(), JsonError> {
        while let Some(c) = self.peek() {
            match c {
                b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' => self.pos += 1,
                b'#' => self.skip_line(),
                b'/' if self.s.get(self.pos + 1) == Some(&b'/') => self.skip_line(),
                b'/' if self.s.get(self.pos + 1) == Some(&b'*') => {
                    let end = self.s[self.pos + 2..].windows(2).position(|w| w == b"*/");
                    match end {
                        Some(e) => self.pos += 2 + e + 2,
                        None => return self.err("unterminated comment"),
                    }
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn skip_line(&mut self) {
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == b'\n' || c == b'\r' {
                break;
            }
        }
    }

    fn value(&mut self) -> Result<Value, JsonError> {
        self.ws()?;
        match self.peek() {
            Some(b'{') => self.nested(Self::object),
            Some(b'[') => self.nested(Self::array),
            Some(q @ (b'"' | b'\'')) => self.quoted(q).map(Value::String),
            Some(_) => self.literal(),
            None => self.err("unexpected end of input"),
        }
    }

    fn nested(&mut self, f: fn(&mut Self) -> Result<Value, JsonError>) -> Result<Value, JsonError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.err("nested too deeply");
        }
        let v = f(self);
        self.depth -= 1;
        v
    }

    fn object(&mut self) -> Result<Value, JsonError> {
        let mut map = Map::new();
        self.members(|k, v| _ = map.insert(k, v))?;
        Ok(Value::Object(map))
    }

    fn members(&mut self, mut member: impl FnMut(String, Value)) -> Result<(), JsonError> {
        self.pos += 1;
        loop {
            self.ws()?;
            match self.peek() {
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(b',' | b';') => {
                    self.pos += 1;
                    continue;
                }
                None => return self.err("unterminated object"),
                _ => {}
            }
            let name = match self.peek() {
                Some(q @ (b'"' | b'\'')) => self.quoted(q)?,
                _ => self.unquoted().to_owned(),
            };
            self.ws()?;
            match self.peek() {
                Some(b':') => self.pos += 1,
                Some(b'=') => self.pos += if self.s.get(self.pos + 1) == Some(&b'>') { 2 } else { 1 },
                _ => return self.err("expected ':' after name"),
            }
            let value = self.value()?;
            member(name, value);
            self.ws()?;
            if !matches!(self.peek(), Some(b',' | b';' | b'}')) {
                return self.err("expected ',' or '}'");
            }
        }
    }

    fn array(&mut self) -> Result<Value, JsonError> {
        self.pos += 1;
        let mut items = Vec::new();
        // whether the last thing read was a value, so ",," and "[," yield nulls as in Gson
        let mut after_value = false;
        loop {
            self.ws()?;
            match self.peek() {
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                Some(b',' | b';') => {
                    self.pos += 1;
                    if !after_value {
                        items.push(Value::Null);
                    }
                    after_value = false;
                }
                None => return self.err("unterminated array"),
                _ if after_value => return self.err("expected ',' or ']'"),
                _ => {
                    items.push(self.value()?);
                    after_value = true;
                }
            }
        }
    }

    fn quoted(&mut self, quote: u8) -> Result<String, JsonError> {
        self.pos += 1;
        let mut out = Vec::new();
        loop {
            let Some(c) = self.peek() else { return self.err("unterminated string") };
            self.pos += 1;
            match c {
                c if c == quote => break,
                b'\\' => {
                    let Some(e) = self.peek() else { return self.err("unterminated escape") };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let ch = self.unicode_escape()?;
                            out.extend(ch.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                c => out.push(c),
            }
        }
        String::from_utf8(out).or_else(|_| self.err("invalid UTF-8 in string"))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let digits = self.s.get(self.pos..self.pos + 4).ok_or(JsonError { pos: self.pos, msg: "short \\u escape" })?;
        let v = std::str::from_utf8(digits).ok().and_then(|d| u32::from_str_radix(d, 16).ok());
        self.pos += 4;
        v.ok_or(JsonError { pos: self.pos, msg: "bad \\u escape" })
    }

    fn unicode_escape(&mut self) -> Result<char, JsonError> {
        let hi = self.hex4()?;
        if (0xD800..0xDC00).contains(&hi) && self.s.get(self.pos..self.pos + 2) == Some(b"\\u") {
            self.pos += 2;
            let lo = self.hex4()?;
            return Ok(char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF)).unwrap_or('\u{fffd}'));
        }
        Ok(char::from_u32(hi).unwrap_or('\u{fffd}'))
    }

    /// Gson's unquoted literal: everything up to a delimiter.
    fn unquoted(&mut self) -> &str {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if matches!(c, b'/' | b'\\' | b';' | b'#' | b'=' | b'{' | b'}' | b'[' | b']' | b':' | b',' | b' ' | b'\t' | b'\x0c' | b'\r' | b'\n') {
                break;
            }
            self.pos += 1;
        }
        std::str::from_utf8(&self.s[start..self.pos]).unwrap_or("")
    }

    fn literal(&mut self) -> Result<Value, JsonError> {
        let lit = self.unquoted();
        if lit.is_empty() {
            return self.err("unexpected character");
        }
        Ok(match lit {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            "null" | "NaN" | "Infinity" | "-Infinity" => Value::Null,
            _ => number(lit).unwrap_or_else(|| Value::String(lit.to_owned())),
        })
    }
}

fn number(lit: &str) -> Option<Value> {
    if let Ok(i) = lit.parse::<i64>() {
        return Some(Value::Number(i.into()));
    }
    let looks_numeric = lit.bytes().next().is_some_and(|c| c.is_ascii_digit() || c == b'-' || c == b'.');
    lit.parse::<f64>().ok().filter(|_| looks_numeric).and_then(Number::from_f64).map(Value::Number)
}

#[cfg(test)]
#[path = "json_tests.rs"]
mod tests;
