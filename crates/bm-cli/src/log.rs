//! BlueMap's `Logger.global` for the CLI: `[LEVEL] message` lines on stdout plus any number of log files.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, PoisonError};

static FILES: Mutex<Vec<File>> = Mutex::new(Vec::new());

/// Adds a log file (`-l`, or core.conf `log.file`); `append` keeps earlier content.
pub fn add_file(path: &Path, append: bool) -> std::io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let file = OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(path)?;
    FILES.lock().unwrap_or_else(PoisonError::into_inner).push(file);
    Ok(())
}

fn log(level: &str, msg: &str) {
    let line = format!("[{level}] {msg}");
    println!("{line}");
    for f in FILES.lock().unwrap_or_else(PoisonError::into_inner).iter_mut() {
        // a failing log file must not stop the render
        let _ = writeln!(f, "{line}");
    }
}

pub fn info(msg: &str) {
    log("INFO", msg);
}

pub fn warn(msg: &str) {
    log("WARNING", msg);
}

pub fn error(msg: &str) {
    log("ERROR", msg);
}
