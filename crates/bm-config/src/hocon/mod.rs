//! HOCON parser matching what BlueMap's Configurate 4.2 loader (typesafe-config inside) accepts and produces.
//!
//! Supported: `#`/`//` comments, `:`/`=`/`{` separators, dotted and quoted keys, quoted/unquoted/`"""` strings,
//! numbers/booleans/null, newline- or comma-separated arrays and fields (trailing commas ok), duplicate-key object
//! merging, value concatenation (strings, lists, objects), `+=`, `${path}`/`${?path}` substitutions with
//! environment-variable fallback and self-references, `include file(...)`/`url(file:...)`/`required(...)`.
//! Plain `include "x"` and `classpath(...)` are skipped, as in BlueMap (see `include.rs`). JSON is a subset.
//! Not supported: `.properties` includes, non-file URL includes, typesafe's delayed merges of an object with a
//! later substitution that resolves to an object.

mod include;
mod lexer;
mod parser;
mod path;
mod raw;
mod resolve;
mod token;

use std::path::Path;

use crate::error::{ConfigError, ParseError};
use crate::value::Map;
use include::IncludeCtx;
use raw::RawObj;

/// Parses and resolves a HOCON/JSON document; `origin` labels error positions. `include file(...)` resolves
/// against the process working directory, like Java's relative paths.
pub fn parse_str(src: &str, origin: &str) -> Result<Map, ParseError> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let mut ctx = IncludeCtx { cwd, depth: 0 };
    let mut root = RawObj::default();
    parse_into(src, origin, &mut root, &[], &mut ctx)?;
    resolve::resolve(&root, origin)
}

/// Reads and parses a config file (`.conf` and `.json` alike, as BlueMap does).
pub fn parse_file(path: &Path) -> Result<Map, ConfigError> {
    let bytes = std::fs::read(path).map_err(|source| ConfigError::Read { path: path.to_owned(), source })?;
    let origin = path.display().to_string();
    let src = String::from_utf8(bytes).map_err(|e| ParseError {
        origin: origin.clone(),
        line: 1,
        col: 1,
        message: format!("file is not valid UTF-8 (invalid byte at offset {})", e.utf8_error().valid_up_to()),
    })?;
    Ok(parse_str(&src, &origin)?)
}

pub(crate) fn parse_into(
    src: &str,
    origin: &str,
    target: &mut RawObj,
    abs: &[String],
    ctx: &mut IncludeCtx,
) -> Result<(), ParseError> {
    let toks = lexer::Lexer::new(src).tokenize().map_err(|(line, col, message)| ParseError {
        origin: origin.to_owned(),
        line,
        col,
        message,
    })?;
    parser::Parser::new(toks, origin, abs.to_vec(), ctx).parse_document(target, abs)
}
