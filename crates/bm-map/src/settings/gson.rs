//! Compact JSON exactly as BlueMap's Gson 2.8.9 instances print it: no whitespace, null fields omitted,
//! HTML-safe string escaping, numbers through Java's `toString`.

pub(super) struct JsonObject {
    buf: String,
}

impl JsonObject {
    pub fn new() -> Self {
        Self { buf: String::from("{") }
    }

    fn key(&mut self, key: &str) {
        if self.buf.len() > 1 {
            self.buf.push(',');
        }
        push_string(&mut self.buf, key);
        self.buf.push(':');
    }

    pub fn raw(mut self, key: &str, json: &str) -> Self {
        self.key(key);
        self.buf.push_str(json);
        self
    }

    pub fn string(mut self, key: &str, value: &str) -> Self {
        self.key(key);
        push_string(&mut self.buf, value);
        self
    }

    /// `serializeNulls` is off: a null field is left out entirely.
    pub fn opt_string(self, key: &str, value: Option<&str>) -> Self {
        match value {
            Some(v) => self.string(key, v),
            None => self,
        }
    }

    pub fn int(self, key: &str, value: impl Into<i64>) -> Self {
        self.raw(key, &value.into().to_string())
    }

    pub fn bool(self, key: &str, value: bool) -> Self {
        self.raw(key, if value { "true" } else { "false" })
    }

    pub fn finish(mut self) -> String {
        self.buf.push('}');
        self.buf
    }
}

pub(super) fn int_array(values: &[i32]) -> String {
    array(values.iter().map(|v| v.to_string()))
}

pub(super) fn string_array<S: AsRef<str>>(values: &[S]) -> String {
    array(values.iter().map(|v| {
        let mut s = String::new();
        push_string(&mut s, v.as_ref());
        s
    }))
}

pub(super) fn array(items: impl Iterator<Item = String>) -> String {
    let items: Vec<String> = items.collect();
    format!("[{}]", items.join(","))
}

/// `JsonWriter.string` with `htmlSafe` (Gson's default).
fn push_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{c}' => out.push_str("\\f"),
            '<' | '>' | '&' | '=' | '\'' | '\u{2028}' | '\u{2029}' | '\0'..='\u{1f}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_like_gson() {
        let mut s = String::new();
        push_string(&mut s, "a\"b\\c\td<>&='\u{1}\u{7f}\u{2028}\u{e9}");
        let esc: String = ["003c", "003e", "0026", "003d", "0027", "0001"].iter().map(|h| format!("\\u{h}")).collect();
        assert_eq!(s, format!("\"a\\\"b\\\\c\\td{esc}\u{7f}\\u2028\u{e9}\""));
    }

    #[test]
    fn object_layout() {
        let json = JsonObject::new().string("a", "x").opt_string("n", None).int("i", -3).bool("b", true).finish();
        assert_eq!(json, r#"{"a":"x","i":-3,"b":true}"#);
        assert_eq!(int_array(&[1, 2]), "[1,2]");
        assert_eq!(string_array::<&str>(&[]), "[]");
    }
}
