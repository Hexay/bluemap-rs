//! BlueMap's `Logger.global` for the CLI: `PrintStreamLogger` on stdout/stderr plus any number of
//! `Logger.file` (`java.util.logging.FileHandler` + `LogFormatter`) files.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use bm_web::{Arg, JavaFormat};
use chrono::Local;

static FILES: Mutex<Vec<File>> = Mutex::new(Vec::new());

const LINE_SEP: &str = if cfg!(windows) { "\r\n" } else { "\n" };

#[derive(Clone, Copy)]
enum Level {
    Info,
    Warning,
    Error,
}

impl Level {
    /// `PrintStreamLogger`'s name.
    fn console(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warning => "WARNING",
            Level::Error => "ERROR",
        }
    }

    /// The `java.util.logging.Level` `JavaLogger` maps to.
    fn jul(self) -> &'static str {
        match self {
            Level::Info => "INFO",
            Level::Warning => "WARNING",
            Level::Error => "SEVERE",
        }
    }
}

/// `-l <file>`: `Logger.file(Path.of(file), append)`.
pub fn add_file(pattern: &str, append: bool) -> std::io::Result<PathBuf> {
    open(file_handler_path(pattern), append)
}

/// core.conf / webserver.conf `log.file`: `String.format(file, now)` first, then like [`add_file`].
pub fn add_formatted_file(pattern: &str, append: bool) -> anyhow::Result<PathBuf> {
    let formatted = JavaFormat::compile(pattern)?.format(&[Arg::Time(Local::now())]);
    Ok(add_file(&formatted, append)?)
}

fn open(path: PathBuf, append: bool) -> std::io::Result<PathBuf> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&path)?;
    FILES.lock().unwrap_or_else(PoisonError::into_inner).push(file);
    Ok(path)
}

/// `FileHandler`'s pattern: `%t` temp dir, `%h` user home, `%g` generation and `%u` unique number (both 0 for a
/// single unlocked file), `%%` a percent sign; `/` is the separator.
pub fn file_handler_path(pattern: &str) -> PathBuf {
    let mut out = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(if c == '/' { std::path::MAIN_SEPARATOR } else { c });
            continue;
        }
        match chars.peek() {
            Some('t') => out.push_str(&std::env::temp_dir().to_string_lossy()),
            Some('h') => out.push_str(&home_dir().to_string_lossy()),
            Some('g' | 'u') => out.push('0'),
            Some('%') => out.push('%'),
            _ => {
                out.push('%');
                continue;
            }
        }
        chars.next();
    }
    let trimmed = out.trim_end_matches(std::path::MAIN_SEPARATOR);
    PathBuf::from(if trimmed.is_empty() { &out } else { trimmed })
}

fn home_dir() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

fn log(level: Level, msg: &str) {
    let line = format!("[{} {}] {msg}", Local::now().format("%H:%M:%S"), level.console());
    if matches!(level, Level::Error) {
        eprintln!("{line}")
    } else {
        println!("{line}")
    }
    let mut files = FILES.lock().unwrap_or_else(PoisonError::into_inner);
    if files.is_empty() {
        return;
    }
    let line = format!("[{}][{}] {msg}{LINE_SEP}", Local::now().format("%Y-%m-%d %H:%M:%S"), level.jul());
    for f in files.iter_mut() {
        // a failing log file must not stop the render
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn info(msg: &str) {
    log(Level::Info, msg);
}

pub fn warn(msg: &str) {
    log(Level::Warning, msg);
}

pub fn error(msg: &str) {
    log(Level::Error, msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_handler_pattern() {
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(file_handler_path("logs/debug.log"), PathBuf::from(format!("logs{sep}debug.log")));
        assert_eq!(file_handler_path("a%g_%u%%.log"), PathBuf::from("a0_0%.log"));
        assert_eq!(file_handler_path("%x"), PathBuf::from("%x"));
        assert!(file_handler_path("%t/x.log").starts_with(std::env::temp_dir()));
    }
}
