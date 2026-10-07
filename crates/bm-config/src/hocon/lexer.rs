//! Tokenizer following typesafe-config's `Tokenizer` (the HOCON implementation Configurate 4.2 embeds).

use super::path::build_path;
use super::token::{Tok, Token};
use crate::value::Value;

/// Characters that end unquoted text; those without a token meaning are errors outside quotes.
const NOT_IN_UNQUOTED: &str = "$\"{}[]:=,+#`^?!@*&\\";

/// typesafe `ConfigNumber.newNumber(double)`: a whole double becomes an int (`1.0`, `1e3` → `ConfigInt`).
fn number_from_double(d: f64) -> Value {
    let l = d as i64;
    if l as f64 == d { Value::Int(l) } else { Value::Float(d) }
}

pub(crate) fn is_ws(c: char) -> bool {
    c != '\n' && (c.is_whitespace() || c == '\u{feff}' || ('\u{1c}'..='\u{1f}').contains(&c))
}

pub(crate) struct Lexer {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    col: usize,
}

/// Error position plus message; the parser attaches the origin.
pub(crate) type LexResult<T> = Result<T, (usize, usize, String)>;

impl Lexer {
    pub(crate) fn new(src: &str) -> Self {
        Self { chars: src.chars().collect(), pos: 0, line: 1, col: 1 }
    }

    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.pos + ahead).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek(0)?;
        self.pos += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn err<T>(&self, msg: impl Into<String>) -> LexResult<T> {
        Err((self.line, self.col, msg.into()))
    }

    pub(crate) fn tokenize(mut self) -> LexResult<Vec<Token>> {
        let mut out = Vec::new();
        loop {
            let (line, col) = (self.line, self.col);
            let Some(c) = self.peek(0) else {
                out.push(Token { tok: Tok::Eof, line, col });
                return Ok(out);
            };
            let tok = match c {
                '\n' => {
                    self.bump();
                    Tok::Newline
                }
                '#' => {
                    self.skip_comment();
                    continue;
                }
                '/' if self.peek(1) == Some('/') => {
                    self.skip_comment();
                    continue;
                }
                c if is_ws(c) => Tok::Ws(self.take_while(is_ws)),
                '"' => self.quoted()?,
                ',' | ':' | '=' | '{' | '}' | '[' | ']' => {
                    self.bump();
                    match c {
                        ',' => Tok::Comma,
                        ':' => Tok::Colon,
                        '=' => Tok::Equals,
                        '{' => Tok::LBrace,
                        '}' => Tok::RBrace,
                        '[' => Tok::LBracket,
                        _ => Tok::RBracket,
                    }
                }
                '+' if self.peek(1) == Some('=') => {
                    self.bump();
                    self.bump();
                    Tok::PlusEquals
                }
                '$' if self.peek(1) == Some('{') => self.substitution()?,
                '0'..='9' | '-' => self.number()?,
                c if NOT_IN_UNQUOTED.contains(c) => return self.reserved(c),
                _ => Tok::Unquoted(self.unquoted()),
            };
            out.push(Token { tok, line, col });
        }
    }

    fn reserved<T>(&self, c: char) -> LexResult<T> {
        let hint = match c {
            '+' => " ('+' is only valid as part of '+=')",
            '$' => " ('$' is only valid as the start of a substitution '${...}')",
            _ => "",
        };
        self.err(format!(
            "reserved character '{c}' is not allowed outside quotes{hint}; enclose the value in double quotes"
        ))
    }

    fn take_while(&mut self, f: impl Fn(char) -> bool) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek(0).filter(|&c| f(c)) {
            s.push(c);
            self.bump();
        }
        s
    }

    fn skip_comment(&mut self) {
        self.take_while(|c| c != '\n');
    }

    fn unquoted(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek(0) {
            if c == '\n' || is_ws(c) || NOT_IN_UNQUOTED.contains(c) || (c == '/' && self.peek(1) == Some('/')) {
                break;
            }
            s.push(c);
            self.bump();
        }
        s
    }

    /// typesafe: digits/sign/exponent run; if it doesn't parse as a Java long/double it is unquoted text instead.
    fn number(&mut self) -> LexResult<Tok> {
        let text = self.take_while(|c| "0123456789eE+-.".contains(c));
        let value = if text.contains(['.', 'e', 'E']) {
            text.parse::<f64>().ok().map(number_from_double)
        } else {
            text.parse::<i64>().ok().map(Value::Int)
        };
        match value {
            Some(value) => Ok(Tok::Number { text, value }),
            None if text.contains('+') => self.reserved('+'),
            None => Ok(Tok::Unquoted(text)),
        }
    }

    fn quoted(&mut self) -> LexResult<Tok> {
        self.bump();
        if self.peek(0) == Some('"') && self.peek(1) == Some('"') {
            self.bump();
            self.bump();
            return self.triple_quoted();
        }
        let mut s = String::new();
        loop {
            if self.peek(0) == Some('\n') {
                return self.err("unterminated quoted string: a newline is not allowed inside \"...\"; use \\n or a \"\"\"multi-line string\"\"\"");
            }
            match self.bump() {
                None => return self.err("unterminated quoted string (missing closing '\"')"),
                Some('"') => return Ok(Tok::Quoted(s)),
                Some('\\') => s.push(self.escape()?),
                Some(c) if (c as u32) < 0x20 => {
                    return self
                        .err(format!("control character U+{:04X} must be escaped in a quoted string", c as u32));
                }
                Some(c) => s.push(c),
            }
        }
    }

    fn escape(&mut self) -> LexResult<char> {
        Ok(match self.bump() {
            Some('"') => '"',
            Some('\\') => '\\',
            Some('/') => '/',
            Some('b') => '\u{8}',
            Some('f') => '\u{c}',
            Some('n') => '\n',
            Some('r') => '\r',
            Some('t') => '\t',
            Some('u') => {
                let hex: String = (0..4).filter_map(|_| self.bump()).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(c) if hex.len() == 4 => c,
                    _ => return self.err(format!("invalid unicode escape '\\u{hex}' (expected 4 hex digits)")),
                }
            }
            Some(c) => {
                return self.err(format!(
                    "invalid escape '\\{c}' in quoted string (valid: \\\" \\\\ \\/ \\b \\f \\n \\r \\t \\uXXXX)"
                ));
            }
            None => return self.err("unterminated quoted string"),
        })
    }

    /// Ends at the first run of 3+ quotes; quotes beyond the last three belong to the content.
    fn triple_quoted(&mut self) -> LexResult<Tok> {
        let mut s = String::new();
        loop {
            match self.peek(0) {
                None => return self.err("unterminated multi-line string (missing closing '\"\"\"')"),
                Some('"') => {
                    let run = self.take_while(|c| c == '"').len();
                    if run >= 3 {
                        s.extend(std::iter::repeat_n('"', run - 3));
                        return Ok(Tok::Quoted(s));
                    }
                    s.extend(std::iter::repeat_n('"', run));
                }
                Some(c) => {
                    s.push(c);
                    self.bump();
                }
            }
        }
    }

    fn substitution(&mut self) -> LexResult<Tok> {
        let (line, col) = (self.line, self.col);
        self.bump();
        self.bump();
        let optional = self.peek(0) == Some('?');
        if optional {
            self.bump();
        }
        let mut pieces = Vec::new();
        loop {
            match self.peek(0) {
                None | Some('\n') => return Err((line, col, "substitution '${' is not closed with '}'".into())),
                Some('}') => {
                    self.bump();
                    break;
                }
                Some('"') => match self.quoted()? {
                    Tok::Quoted(s) => pieces.push((s, true)),
                    _ => return Err((line, col, "multi-line strings are not allowed in substitutions".into())),
                },
                Some(_) => {
                    let text = self.take_while(|c| !matches!(c, '}' | '"' | '\n'));
                    pieces.push((text, false));
                }
            }
        }
        if let Some((text, false)) = pieces.first_mut() {
            *text = text.trim_start().to_owned();
        }
        if let Some((text, false)) = pieces.last_mut() {
            *text = text.trim_end().to_owned();
        }
        match build_path(&pieces) {
            Ok(path) => Ok(Tok::Subst { path, optional }),
            Err(msg) => Err((line, col, format!("invalid substitution: {msg}"))),
        }
    }
}
