//! Ordered JSON tree, the `JsonElement` view MarkerGson's adapters read. Numbers keep their literal text like
//! Gson's `LazilyParsedNumber`, since string and int coercions depend on it.

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn kind(&self) -> &'static str {
        match self {
            Json::Null => "null",
            Json::Bool(_) => "a boolean",
            Json::Num(_) => "a number",
            Json::Str(_) => "a string",
            Json::Arr(_) => "an array",
            Json::Obj(_) => "an object",
        }
    }

    /// Member lookup with Gson's `JsonObject` semantics: a repeated key keeps the last value.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(members) => members.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

pub(super) fn parse(text: &str) -> Result<Json, String> {
    let mut p = Parser { s: text.as_bytes(), pos: 0 };
    let value = p.value()?;
    p.ws();
    if p.pos != p.s.len() {
        return Err(p.err("trailing characters"));
    }
    Ok(value)
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn err(&self, what: &str) -> String {
        format!("{what} at byte {}", self.pos)
    }

    fn ws(&mut self) {
        while self.s.get(self.pos).is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        let hit = self.s.get(self.pos) == Some(&c);
        self.pos += hit as usize;
        hit
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.s[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.err("unexpected token"))
        }
    }

    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.s.get(self.pos) {
            Some(b'{') => self
                .container(b'}', |p| {
                    let key = p.string()?;
                    if !p.eat(b':') {
                        return Err(p.err("expected ':'"));
                    }
                    Ok((key, p.value()?))
                })
                .map(Json::Obj),
            Some(b'[') => self.container(b']', Parser::value).map(Json::Arr),
            Some(b'"') => self.string().map(Json::Str),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.err("expected a value")),
        }
    }

    fn container<T>(
        &mut self,
        close: u8,
        mut item: impl FnMut(&mut Self) -> Result<T, String>,
    ) -> Result<Vec<T>, String> {
        self.pos += 1;
        let mut items = Vec::new();
        if self.eat(close) {
            return Ok(items);
        }
        loop {
            items.push(item(self)?);
            if self.eat(close) {
                return Ok(items);
            }
            if !self.eat(b',') {
                return Err(self.err("expected ',' or a closing bracket"));
            }
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        let digits = |p: &mut Self| {
            let from = p.pos;
            while p.s.get(p.pos).is_some_and(u8::is_ascii_digit) {
                p.pos += 1;
            }
            p.pos > from
        };
        self.pos += (self.s[self.pos] == b'-') as usize;
        let mut ok = digits(self);
        if self.s.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            ok &= digits(self);
        }
        if matches!(self.s.get(self.pos), Some(b'e' | b'E')) {
            self.pos += 1;
            self.pos += matches!(self.s.get(self.pos), Some(b'+' | b'-')) as usize;
            ok &= digits(self);
        }
        if !ok {
            return Err(self.err("malformed number"));
        }
        Ok(Json::Num(String::from_utf8_lossy(&self.s[start..self.pos]).into_owned()))
    }

    fn string(&mut self) -> Result<String, String> {
        if !self.eat(b'"') {
            return Err(self.err("expected a string"));
        }
        let mut out = Vec::new();
        loop {
            let Some(&c) = self.s.get(self.pos) else { return Err(self.err("unterminated string")) };
            self.pos += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|_| self.err("invalid UTF-8")),
                b'\\' => {
                    let Some(&e) = self.s.get(self.pos) else { return Err(self.err("unterminated escape")) };
                    self.pos += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => self.unicode_escape()?,
                        _ => return Err(self.err("invalid escape")),
                    };
                    out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                }
                c => out.push(c),
            }
        }
    }

    /// `\uXXXX`, joining a surrogate pair; a lone surrogate (legal in Java strings) becomes U+FFFD.
    fn unicode_escape(&mut self) -> Result<char, String> {
        let hi = self.hex4()?;
        if (0xd800..0xdc00).contains(&hi) && self.s[self.pos..].starts_with(b"\\u") {
            let save = self.pos;
            self.pos += 2;
            let lo = self.hex4()?;
            if (0xdc00..0xe000).contains(&lo) {
                let c = 0x10000 + ((hi - 0xd800) << 10) + (lo - 0xdc00);
                return Ok(char::from_u32(c).expect("valid surrogate pair"));
            }
            self.pos = save;
        }
        Ok(char::from_u32(hi).unwrap_or('\u{fffd}'))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let hex = self.s.get(self.pos..self.pos + 4).and_then(|h| std::str::from_utf8(h).ok());
        let value = hex.and_then(|h| u32::from_str_radix(h, 16).ok()).ok_or_else(|| self.err("invalid \\u escape"))?;
        self.pos += 4;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_order_and_number_text() {
        let v = parse(r#" {"b": [1.50, -0, 1E3], "a": {"x": null, "y": true}, "s": "\u00e9\ud83d\ude00\n"} "#).unwrap();
        let Json::Obj(members) = &v else { panic!() };
        assert_eq!(members.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["b", "a", "s"]);
        assert_eq!(
            v.get("b"),
            Some(&Json::Arr(vec![Json::Num("1.50".into()), Json::Num("-0".into()), Json::Num("1E3".into())]))
        );
        assert_eq!(v.get("s"), Some(&Json::Str("\u{e9}\u{1f600}\n".into())));
        assert_eq!(v.get("a").and_then(|a| a.get("y")), Some(&Json::Bool(true)));
    }

    #[test]
    fn rejects_garbage() {
        for bad in ["", "{", "[1,]", "{\"a\" 1}", "01x", "1.", "\"\\x\"", "{} {}", "NaN"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
