//! Vanilla text-component JSON with BlueMap's command palette (`commands/TextFormat.java`).

use serde_json::{Value, json};

pub const BASE: &str = "#aaaaaa";
pub const HIGHLIGHT: &str = "#ffffff";
pub const TITLE: &str = "#4488ff";
pub const POSITIVE: &str = "#88ff88";
pub const NEGATIVE: &str = "#ff8888";
pub const INFO: &str = "#ffff88";
pub const FROZEN: &str = "#aaccff";

pub fn span(text: impl Into<String>, color: &str) -> Value {
    json!({"text": text.into(), "color": color})
}

/// `format("a % b", x)`: base-coloured text with `%` replaced by highlighted arguments in order.
pub fn format(template: &str, args: &[&str]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut parts = template.split('%');
    if let Some(first) = parts.next().filter(|s| !s.is_empty()) {
        out.push(span(first, BASE));
    }
    for (i, part) in parts.enumerate() {
        out.push(span(args.get(i).copied().unwrap_or("?"), HIGHLIGHT));
        if !part.is_empty() {
            out.push(span(part, BASE));
        }
    }
    out
}

/// Lines of spans joined with newlines into one component.
pub fn lines(lines: Vec<Vec<Value>>) -> Value {
    let mut extra = Vec::new();
    for (i, line) in lines.into_iter().enumerate() {
        if i > 0 {
            extra.push(json!("\n"));
        }
        extra.extend(line);
    }
    json!({"text": "", "extra": extra})
}

/// `paragraph(title, content)`: a bold "BlueMap <title> >" header, then the indented lines.
pub fn paragraph(title: &str, body: Vec<Vec<Value>>) -> Value {
    let mut all = vec![vec![
        json!({"text": "BlueMap ", "color": TITLE, "bold": true}),
        json!({"text": title, "color": HIGHLIGHT, "bold": false}),
        json!({"text": " >", "color": TITLE, "bold": true}),
    ]];
    all.extend(body.into_iter().map(|mut l| {
        l.insert(0, json!(" "));
        l
    }));
    lines(all)
}

pub fn one(text: &str, color: &str) -> Value {
    lines(vec![vec![span(text, color)]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_fills_placeholders() {
        let spans = format("Map % is %", &["world", "frozen"]);
        let texts: Vec<&str> = spans.iter().map(|v| v["text"].as_str().unwrap()).collect();
        assert_eq!(texts, ["Map ", "world", " is ", "frozen"]);
        assert_eq!(spans[1]["color"], HIGHLIGHT);
    }
}
