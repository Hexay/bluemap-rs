//! Token stream → unresolved document. Objects are parsed straight into their place in the document so
//! duplicate keys merge and self-references see earlier values (typesafe-config semantics).

use super::include::IncludeCtx;
use super::path::build_path;
use super::raw::{Raw, RawObj, Subst};
use super::token::{Tok, Token};
use crate::error::ParseError;
use crate::value::Value;

pub(crate) struct Parser<'a> {
    toks: Vec<Token>,
    pos: usize,
    origin: &'a str,
    /// Absolute location of the include this source was pulled in by (empty for the main file).
    subst_prefix: Vec<String>,
    pub(super) ctx: &'a mut IncludeCtx,
}

pub(super) type PResult<T> = Result<T, ParseError>;

impl<'a> Parser<'a> {
    pub(crate) fn new(toks: Vec<Token>, origin: &'a str, subst_prefix: Vec<String>, ctx: &'a mut IncludeCtx) -> Self {
        Self { toks, pos: 0, origin, subst_prefix, ctx }
    }

    pub(super) fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    pub(super) fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    pub(crate) fn err<T>(&self, msg: impl Into<String>) -> PResult<T> {
        let t = &self.toks[self.pos];
        Err(self.error_at(t.line, t.col, msg))
    }

    pub(crate) fn error_at(&self, line: usize, col: usize, msg: impl Into<String>) -> ParseError {
        ParseError { origin: self.origin.to_owned(), line, col, message: msg.into() }
    }

    pub(super) fn skip_ws(&mut self) {
        while matches!(self.peek(), Tok::Ws(_)) {
            self.bump();
        }
    }

    fn skip_ws_nl(&mut self) {
        while matches!(self.peek(), Tok::Ws(_) | Tok::Newline) {
            self.bump();
        }
    }

    /// Parses a whole document into `target` (the root, or the object an include sits in).
    pub(crate) fn parse_document(&mut self, target: &mut RawObj, abs: &[String]) -> PResult<()> {
        self.skip_ws_nl();
        match self.peek() {
            Tok::LBrace => {
                let open = self.bump();
                self.parse_object_body(target, abs, Some(open.line))?;
                self.skip_ws_nl();
                match self.peek() {
                    Tok::Eof => Ok(()),
                    other => {
                        self.err(format!("unexpected {} after the closing '}}' of the document", other.describe()))
                    }
                }
            }
            Tok::LBracket => self.err("the document root must be an object, not a list"),
            _ => self.parse_object_body(target, abs, None),
        }
    }

    /// Fields until the matching '}' (`braced` = line of the '{') or end of input.
    fn parse_object_body(&mut self, target: &mut RawObj, abs: &[String], braced: Option<usize>) -> PResult<()> {
        loop {
            self.skip_ws_nl();
            match (self.peek(), braced) {
                (Tok::RBrace, Some(_)) => {
                    self.bump();
                    return Ok(());
                }
                (Tok::Eof, None) => return Ok(()),
                (Tok::Eof, Some(line)) => {
                    return self
                        .err(format!("end of file inside the object opened with '{{' at line {line} (missing '}}')"));
                }
                (Tok::RBrace, None) => return self.err("unbalanced '}' with no matching '{'"),
                (Tok::RBracket, _) => return self.err("unbalanced ']' with no matching '['"),
                (Tok::Unquoted(s), _) if s == "include" => self.parse_include(target, abs)?,
                _ => self.parse_field(target, abs)?,
            }
            self.skip_ws();
            match self.peek() {
                Tok::Comma => {
                    self.bump();
                }
                Tok::Newline | Tok::Eof | Tok::RBrace => {}
                other => {
                    let hint = match other {
                        Tok::Colon | Tok::Equals => {
                            " (if it is part of a value like a URL or time, enclose the value in double quotes)"
                        }
                        _ => "",
                    };
                    return self
                        .err(format!("expected ',' or a newline after a value, got {}{hint}", other.describe()));
                }
            }
        }
    }

    fn parse_field(&mut self, target: &mut RawObj, abs: &[String]) -> PResult<()> {
        let start = self.toks[self.pos].clone();
        let mut pieces = Vec::new();
        loop {
            match self.peek() {
                Tok::Unquoted(s) | Tok::Ws(s) | Tok::Number { text: s, .. } => pieces.push((s.clone(), false)),
                Tok::Quoted(s) => pieces.push((s.clone(), true)),
                _ => break,
            }
            self.bump();
        }
        if pieces.is_empty() {
            return self.err(format!("expected a key, got {}", self.peek().describe()));
        }
        let path = build_path(&pieces).map_err(|m| self.error_at(start.line, start.col, m))?;
        let key = path.join(".");
        let append = match self.peek() {
            Tok::Colon | Tok::Equals => false,
            Tok::PlusEquals => true,
            Tok::LBrace => return self.parse_object_value(target, &path, abs),
            Tok::Newline | Tok::Eof => {
                return self.err(format!("key '{key}' has no value (expected ':' or '=' and a value)"));
            }
            other => {
                return self.err(format!("key '{key}' must be followed by ':', '=' or '{{', got {}", other.describe()));
            }
        };
        self.bump();
        self.skip_ws_nl();
        if !append && matches!(self.peek(), Tok::LBrace) {
            return self.parse_object_value(target, &path, abs);
        }
        let abs_field: Vec<String> = abs.iter().chain(&path).cloned().collect();
        let mut value = self.parse_value(&abs_field, &key)?;
        if append {
            let self_ref = Raw::Subst(Subst {
                path: abs_field.clone(),
                optional: true,
                prefix: vec![],
                line: start.line,
                col: start.col,
            });
            value = Raw::Concat { pieces: vec![self_ref, Raw::List(vec![value])], line: start.line, col: start.col };
        }
        target.set(&path, value, abs);
        Ok(())
    }

    /// `key { ... }`: parsed in place into the existing object; anything concatenated after it is applied on top.
    fn parse_object_value(&mut self, target: &mut RawObj, path: &[String], abs: &[String]) -> PResult<()> {
        let abs_field: Vec<String> = abs.iter().chain(path).cloned().collect();
        let open = self.bump();
        self.parse_object_body(target.object_at(path), &abs_field, Some(open.line))?;
        loop {
            let save = self.pos;
            self.skip_ws();
            if matches!(self.peek(), Tok::LBrace) {
                let open = self.bump();
                self.parse_object_body(target.object_at(path), &abs_field, Some(open.line))?;
                continue;
            }
            if self.at_value_end() {
                self.pos = save;
                return Ok(());
            }
            let mut pieces = vec![Raw::Obj(target.object_at(path).clone())];
            match self.parse_value(&abs_field, &path.join("."))? {
                Raw::Concat { pieces: more, .. } => pieces.extend(more),
                other => pieces.push(other),
            }
            target.set(path, Raw::Concat { pieces, line: open.line, col: open.col }, abs);
            return Ok(());
        }
    }

    fn at_value_end(&self) -> bool {
        use Tok::*;
        matches!(self.peek(), Newline | Comma | RBrace | RBracket | Eof | Colon | Equals | PlusEquals)
    }

    /// One value: a single piece, or a concatenation of pieces up to the end of the line / ',' / closing bracket.
    pub(crate) fn parse_value(&mut self, abs: &[String], key: &str) -> PResult<Raw> {
        let start = self.toks[self.pos].clone();
        let mut pieces = Vec::new();
        while !self.at_value_end() {
            let piece = match self.peek().clone() {
                Tok::Ws(s) => {
                    self.bump();
                    Raw::Ws(s)
                }
                Tok::Quoted(s) => {
                    self.bump();
                    Raw::Lit { value: Value::String(s.clone()), text: s }
                }
                Tok::Unquoted(s) => {
                    self.bump();
                    let value = match s.as_str() {
                        "true" => Value::Bool(true),
                        "false" => Value::Bool(false),
                        "null" => Value::Null,
                        _ => Value::String(s.clone()),
                    };
                    Raw::Lit { value, text: s }
                }
                Tok::Number { text, value } => {
                    self.bump();
                    Raw::Lit { value, text }
                }
                Tok::Subst { path, optional } => {
                    let t = self.bump();
                    let prefix = self.subst_prefix.clone();
                    Raw::Subst(Subst { path, optional, prefix, line: t.line, col: t.col })
                }
                Tok::LBrace => {
                    let open = self.bump();
                    let mut obj = RawObj::default();
                    self.parse_object_body(&mut obj, abs, Some(open.line))?;
                    Raw::Obj(obj)
                }
                Tok::LBracket => self.parse_list(abs, key)?,
                _ => unreachable!("value end tokens are excluded above"),
            };
            pieces.push(piece);
        }
        while matches!(pieces.last(), Some(Raw::Ws(_))) {
            pieces.pop();
        }
        while matches!(pieces.first(), Some(Raw::Ws(_))) {
            pieces.remove(0);
        }
        match pieces.len() {
            0 => self.err(format!("expected a value for '{key}', got {}", self.peek().describe())),
            1 => Ok(pieces.pop().unwrap()),
            _ => Ok(Raw::Concat { pieces, line: start.line, col: start.col }),
        }
    }

    fn parse_list(&mut self, abs: &[String], key: &str) -> PResult<Raw> {
        let open = self.bump();
        let mut items = Vec::new();
        loop {
            self.skip_ws_nl();
            match self.peek() {
                Tok::RBracket => {
                    self.bump();
                    return Ok(Raw::List(items));
                }
                Tok::Eof => {
                    return self.err(format!(
                        "end of file inside the list opened with '[' at line {} (missing ']')",
                        open.line
                    ));
                }
                Tok::Comma => return self.err("unexpected ',' in a list (two commas in a row, or a leading comma)"),
                _ => {}
            }
            items.push(self.parse_value(abs, key)?);
            self.skip_ws();
            match self.peek() {
                Tok::Comma => {
                    self.bump();
                }
                Tok::Newline | Tok::RBracket | Tok::Eof => {}
                other => {
                    return self.err(format!(
                        "list opened at line {} should continue with ',' or end with ']', got {}",
                        open.line,
                        other.describe()
                    ));
                }
            }
        }
    }
}
