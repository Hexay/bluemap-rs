//! The subset of `java.util.Formatter` that BlueMap's log settings use: `%[n$|<][-][width][.prec]` with
//! `s S d % n` and the `t`/`T` date-time conversions. Anything else is rejected when the pattern is compiled,
//! where Java would throw on every log call instead.

use chrono::{DateTime, Local};

#[derive(Debug, Clone)]
pub enum Arg {
    Str(String),
    Int(i64),
    Time(DateTime<Local>),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid Java format pattern {pattern:?}: {reason}")]
pub struct FormatError {
    pub pattern: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Lit(String),
    Spec { arg: usize, left: bool, width: usize, prec: Option<usize>, upper: bool, conv: Conv },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Conv {
    Str,
    Dec,
    Time(char),
}

/// A compiled pattern; arguments are 1-based like Java's `n$`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaFormat {
    pieces: Vec<Piece>,
    max_arg: usize,
}

const LINE_SEP: &str = if cfg!(windows) { "\r\n" } else { "\n" };

impl JavaFormat {
    pub fn compile(pattern: &str) -> Result<Self, FormatError> {
        let err = |reason: String| FormatError { pattern: pattern.to_owned(), reason };
        let mut pieces = Vec::new();
        let mut lit = String::new();
        let (mut ordinary, mut last, mut max_arg) = (0usize, 0usize, 0usize);
        let mut chars = pattern.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '%' {
                lit.push(c);
                continue;
            }
            let mut spec = String::new();
            while let Some(&n) = chars.peek() {
                if n.is_ascii_digit() || matches!(n, '$' | '-' | '.' | '<' | '#' | '+' | ' ' | ',' | '(') {
                    spec.push(n);
                    chars.next();
                } else {
                    break;
                }
            }
            let Some(conv) = chars.next() else { return Err(err("dangling '%'".into())) };
            match conv {
                '%' if spec.is_empty() => lit.push('%'),
                'n' if spec.is_empty() => lit.push_str(LINE_SEP),
                's' | 'S' | 'd' | 't' | 'T' => {
                    let conv_char_upper = matches!(conv, 'S' | 'T');
                    let conv = match conv {
                        's' | 'S' => Conv::Str,
                        'd' => Conv::Dec,
                        _ => match chars.next() {
                            Some(t) if time_pattern(t).is_some() => Conv::Time(t),
                            t => return Err(err(format!("unsupported date/time conversion {t:?}"))),
                        },
                    };
                    let (arg, rest) =
                        parse_index(&spec, &mut ordinary, last).ok_or_else(|| err(format!("bad specifier %{spec}")))?;
                    let (left, width, prec) =
                        parse_layout(rest).ok_or_else(|| err(format!("unsupported flags in %{spec}")))?;
                    (last, max_arg) = (arg, max_arg.max(arg));
                    if !lit.is_empty() {
                        pieces.push(Piece::Lit(std::mem::take(&mut lit)));
                    }
                    pieces.push(Piece::Spec { arg, left, width, prec, upper: conv_char_upper, conv });
                }
                other => return Err(err(format!("unsupported conversion '{other}'"))),
            }
        }
        if !lit.is_empty() {
            pieces.push(Piece::Lit(lit));
        }
        Ok(Self { pieces, max_arg })
    }

    pub fn max_arg(&self) -> usize {
        self.max_arg
    }

    /// Formats `args`. Where Java would throw (missing argument, `%d` of a string) this renders `null`/the value;
    /// callers check [`JavaFormat::max_arg`] up front.
    pub fn format(&self, args: &[Arg]) -> String {
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Lit(s) => out.push_str(s),
                Piece::Spec { arg, left, width, prec, upper, conv } => {
                    let mut s = match (args.get(arg - 1), conv) {
                        (None, _) => "null".to_owned(),
                        (Some(Arg::Time(t)), Conv::Time(c)) => format_time(t, *c),
                        (Some(Arg::Str(s)), _) => s.clone(),
                        (Some(Arg::Int(i)), _) => i.to_string(),
                        (Some(Arg::Time(t)), _) => t.to_rfc3339(),
                    };
                    if let Some(p) = prec {
                        s = s.chars().take(*p).collect();
                    }
                    if *upper {
                        s = s.to_uppercase();
                    }
                    let pad = width.saturating_sub(s.chars().count());
                    if *left {
                        out.push_str(&s);
                        out.extend(std::iter::repeat_n(' ', pad));
                    } else {
                        out.extend(std::iter::repeat_n(' ', pad));
                        out.push_str(&s);
                    }
                }
            }
        }
        out
    }
}

fn parse_index<'a>(spec: &'a str, ordinary: &mut usize, last: usize) -> Option<(usize, &'a str)> {
    if let Some(rest) = spec.strip_prefix('<') {
        return (last > 0).then_some((last, rest));
    }
    if let Some(dollar) = spec.find('$') {
        let n: usize = spec[..dollar].parse().ok()?;
        return (n > 0).then_some((n, &spec[dollar + 1..]));
    }
    *ordinary += 1;
    Some((*ordinary, spec))
}

fn parse_layout(rest: &str) -> Option<(bool, usize, Option<usize>)> {
    let (left, rest) = match rest.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, rest),
    };
    let (width, prec) = match rest.split_once('.') {
        Some((w, p)) => (w, Some(p.parse().ok()?)),
        None => (rest, None),
    };
    let width = if width.is_empty() { 0 } else { width.parse().ok()? };
    (!left || width > 0).then_some((left, width, prec))
}

fn time_pattern(c: char) -> Option<&'static str> {
    Some(match c {
        'H' => "%H",
        'I' => "%I",
        'k' => "%-H",
        'l' => "%-I",
        'M' => "%M",
        'S' => "%S",
        'L' => "%3f",
        'N' => "%9f",
        'p' => "%P",
        'z' => "%z",
        'Z' => "%Z",
        's' => "%s",
        'B' => "%B",
        'b' | 'h' => "%b",
        'A' => "%A",
        'a' => "%a",
        'C' => "%C",
        'Y' => "%Y",
        'y' => "%y",
        'j' => "%j",
        'm' => "%m",
        'd' => "%d",
        'e' => "%-d",
        'R' => "%H:%M",
        'T' => "%H:%M:%S",
        'r' => "%I:%M:%S %p",
        'D' => "%m/%d/%y",
        'F' => "%Y-%m-%d",
        'c' => "%a %b %d %H:%M:%S %Z %Y",
        'Q' => "",
        _ => return None,
    })
}

fn format_time(t: &DateTime<Local>, c: char) -> String {
    match c {
        'Q' => t.timestamp_millis().to_string(),
        _ => t.format(time_pattern(c).unwrap_or_default()).to_string(),
    }
}
