//! `include` handling as BlueMap's loader behaves: Configurate parses from a Reader, so typesafe-config resolves
//! plain `include "x"` (and `classpath(...)`) against the classpath, where nothing is found and it is silently
//! skipped (BlueMap issue #504). `file(...)` resolves against the process working directory and works.

use std::path::{Path, PathBuf};

use super::parser::{PResult, Parser};
use super::raw::RawObj;
use super::token::Tok;
use crate::error::ParseError;

const MAX_DEPTH: usize = 50;

pub(crate) struct IncludeCtx {
    pub cwd: PathBuf,
    pub depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Heuristic,
    File,
    Classpath,
    Url,
}

struct Spec {
    kind: Kind,
    name: String,
    required: bool,
}

enum Failure {
    Message(String),
    Nested(ParseError),
}

impl Parser<'_> {
    /// `include "x"`, `include file("x")`, `include required(file("x"))`, …, merged into `target`.
    pub(super) fn parse_include(&mut self, target: &mut RawObj, abs: &[String]) -> PResult<()> {
        let kw = self.bump();
        self.skip_ws();
        let mut opener = String::new();
        if let Tok::Unquoted(s) = self.peek() {
            opener = s.clone();
            self.bump();
            self.skip_ws();
        }
        let Tok::Quoted(name) = self.peek().clone() else {
            let got = self.peek().describe();
            return self.err(format!("'include' must be followed by a quoted file name, got {got}"));
        };
        self.bump();
        self.skip_ws();
        let mut closer = String::new();
        if let Tok::Unquoted(s) = self.peek() {
            closer = s.clone();
            self.bump();
        }
        let spec = Spec::parse(&opener, &closer, name).map_err(|m| self.error_at(kw.line, kw.col, m))?;
        apply(spec, target, abs, self.ctx).map_err(|m| match m {
            Failure::Message(m) => self.error_at(kw.line, kw.col, m),
            Failure::Nested(e) => e,
        })
    }
}

impl Spec {
    /// `opener`/`closer` are the unquoted text around the quoted name, e.g. `required(file(` and `))`.
    fn parse(opener: &str, closer: &str, name: String) -> Result<Spec, String> {
        let (required, rest) = match opener.strip_prefix("required(") {
            Some(rest) => (true, rest),
            None => (false, opener),
        };
        let kind = match rest {
            "" => Kind::Heuristic,
            "file(" => Kind::File,
            "classpath(" => Kind::Classpath,
            "url(" => Kind::Url,
            other => {
                return Err(format!(
                    "unknown include form '{other}' (expected file(...), classpath(...), url(...) or required(...))"
                ));
            }
        };
        let expected_closer = ")".repeat(opener.matches('(').count());
        if closer != expected_closer {
            return Err(format!(
                "include of \"{name}\" is not closed properly: expected '{expected_closer}', got '{closer}'"
            ));
        }
        Ok(Spec { kind, name, required })
    }
}

fn apply(spec: Spec, target: &mut RawObj, abs: &[String], ctx: &mut IncludeCtx) -> Result<(), Failure> {
    let candidates = match spec.kind {
        Kind::Heuristic | Kind::Classpath => vec![],
        Kind::File => candidates(&ctx.cwd.join(&spec.name)),
        Kind::Url => spec.name.strip_prefix("file:").map(|p| candidates(&file_url_path(p))).unwrap_or_default(),
    };
    let found: Vec<PathBuf> = candidates.into_iter().filter(|p| p.is_file()).collect();
    if found.is_empty() {
        return match spec.required {
            true => Err(Failure::Message(format!("required include \"{}\" was not found", spec.name))),
            false => Ok(()),
        };
    }
    if ctx.depth >= MAX_DEPTH {
        return Err(Failure::Message(format!("includes nested more than {MAX_DEPTH} deep (include cycle?)")));
    }
    for path in found {
        let src = std::fs::read_to_string(&path)
            .map_err(|e| Failure::Message(format!("cannot read included file {}: {e}", path.display())))?;
        ctx.depth += 1;
        let result = super::parse_into(&src, &path.display().to_string(), target, abs, ctx);
        ctx.depth -= 1;
        result.map_err(Failure::Nested)?;
    }
    Ok(())
}

/// typesafe-config: a name without extension tries `.conf` and `.json` (`.properties` is not supported here).
fn candidates(path: &Path) -> Vec<PathBuf> {
    match path.extension() {
        Some(_) => vec![path.to_owned()],
        None => vec![path.with_extension("conf"), path.with_extension("json")],
    }
}

fn file_url_path(rest: &str) -> PathBuf {
    let rest = rest.strip_prefix("//").unwrap_or(rest);
    let windows_drive = rest.len() > 2 && rest.starts_with('/') && rest.as_bytes()[2] == b':';
    PathBuf::from(if windows_drive { &rest[1..] } else { rest })
}
