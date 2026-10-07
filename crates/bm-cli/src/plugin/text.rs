//! Vanilla text-component JSON with BlueMap's command palette (`commands/TextFormat.java`).

use serde_json::{Value, json};

pub const BASE: &str = "#aaaaaa";
pub const HIGHLIGHT: &str = "#ffffff";
pub const TITLE: &str = "#4488ff";
pub const POSITIVE: &str = "#88ff88";
pub const NEGATIVE: &str = "#ff8888";
pub const INFO: &str = "#ffff88";
pub const FROZEN: &str = "#aaccff";
pub const WARNING: &str = "#ff8844";

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

/// A placeholder value with its own colour; `""` takes the surrounding colour.
pub type Arg<'a> = (&'a str, &'a str);

pub fn hl(text: &str) -> Arg<'_> {
    (text, HIGHLIGHT)
}

/// `format(template, args).color(color)` for multi-line templates and arguments: one `Vec` per line.
pub fn fill(template: &str, args: &[Arg], color: &str) -> Vec<Vec<Value>> {
    let mut pieces: Vec<(&str, &str)> = Vec::new();
    for (i, part) in template.split('%').enumerate() {
        if i > 0 {
            let (text, c) = args.get(i - 1).copied().unwrap_or(("null", ""));
            pieces.push((text, if c.is_empty() { color } else { c }));
        }
        pieces.push((part, color));
    }
    let mut out = vec![Vec::new()];
    for (text, c) in pieces {
        for (j, segment) in text.split('\n').enumerate() {
            if j > 0 {
                out.push(Vec::new());
            }
            if !segment.is_empty() {
                out.last_mut().expect("non-empty").push(span(segment, c));
            }
        }
    }
    out
}

/// `item(key, value)`: `key: value` with the key in base colour.
pub fn item(key: &str, value: &str) -> Vec<Value> {
    vec![span(format!("{key}: "), BASE), span(value, HIGHLIGHT)]
}

/// `TextFormat.details`: a tree of items under the previous line (`├ `, `│ `, `└ `).
pub fn details(items: Vec<Vec<Vec<Value>>>, color: &str) -> Vec<Vec<Value>> {
    let n = items.len();
    let mut out = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        let (first, rest) = if i + 1 == n { ("└ ", "\u{a0} ") } else { ("├ ", "│ ") };
        for (j, mut line) in item.into_iter().enumerate() {
            line.insert(0, span(if j == 0 { first } else { rest }, color));
            out.push(line);
        }
    }
    out
}

/// `TextFormat.duration`: the largest unit above 1 (days … seconds), one decimal below 2.
pub fn duration(millis: i64) -> String {
    let units = [("days", 86_400_000i64), ("hours", 3_600_000), ("minutes", 60_000), ("seconds", 1000)];
    let (mut name, mut value) = ("seconds", 0.0);
    for (unit, ms) in units {
        (name, value) = (unit, millis as f64 / ms as f64);
        if value > 1.0 {
            break;
        }
    }
    // Java's %.Nf rounds half up; Rust's formatter rounds half to even
    if value < 2.0 && name != "seconds" {
        format!("{:.1} {name}", (value * 10.0).round() / 10.0)
    } else {
        format!("{:.0} {name}", value.round())
    }
}

/// `durationFormat(Instant.ofEpochSecond(secs))`: time since then.
pub fn since(epoch_secs: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    duration(now - epoch_secs * 1000)
}

/// `DATE_TIME_FORMAT` in local time.
pub fn date_time(epoch_secs: i64) -> String {
    chrono::DateTime::from_timestamp(epoch_secs, 0)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default()
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

    #[test]
    fn fill_splits_lines_and_details_draw_a_tree() {
        let lines = fill("a %\nb %", &[hl("x"), ("y\nz", "")], WARNING);
        let texts: Vec<Vec<&str>> =
            lines.iter().map(|l| l.iter().map(|v| v["text"].as_str().unwrap()).collect()).collect();
        assert_eq!(texts, [vec!["a ", "x"], vec!["b ", "y"], vec!["z"]]);
        assert_eq!(lines[2][0]["color"], WARNING);
        let tree = details(vec![vec![item("a", "1")], vec![item("b", "2"), item("c", "3")]], BASE);
        let firsts: Vec<&str> = tree.iter().map(|l| l[0]["text"].as_str().unwrap()).collect();
        assert_eq!(firsts, ["├ ", "└ ", "\u{a0} "]);
    }

    #[test]
    fn durations_like_java() {
        assert_eq!(duration(90_000), "1.5 minutes");
        assert_eq!(duration(3 * 86_400_000), "3 days");
        assert_eq!(duration(500), "1 seconds");
        assert_eq!(duration(5000), "5 seconds");
    }
}
