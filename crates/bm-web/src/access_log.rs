//! `LoggingRequestHandler`: one line per request in the configured Java format, 5xx as warnings. Args: 1 source ip,
//! 2 leftmost `X-Forwarded-For` (else the source), 3 method, 4 `path?query`, 5 version, 6 status code, 7 reason.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{self, Sender};

use chrono::Local;

use crate::WebError;
use crate::javafmt::{Arg, JavaFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warning,
}

impl Level {
    fn name(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warning => "WARNING",
        }
    }
}

pub trait LogSink: Send + Sync {
    fn log(&self, level: Level, message: &str);
}

/// What the log line needs from a request, captured before routing rewrites anything.
#[derive(Debug, Clone)]
pub struct RequestInfo {
    pub source: IpAddr,
    pub forwarded_for: Option<String>,
    pub method: String,
    /// Java logs `path + "?" + rawQuery` even for an empty query, hence `/index.html?`.
    pub address: String,
    pub version: &'static str,
}

pub struct AccessLog {
    format: JavaFormat,
    sinks: Vec<Box<dyn LogSink>>,
}

impl AccessLog {
    pub fn new(format: &str, sinks: Vec<Box<dyn LogSink>>) -> Result<Self, WebError> {
        let format = JavaFormat::compile(format)?;
        if format.max_arg() > 7 {
            return Err(WebError::LogFormatArgs(format.max_arg()));
        }
        Ok(Self { format, sinks })
    }

    pub fn disabled() -> Self {
        Self { format: JavaFormat::compile("").expect("empty pattern"), sinks: Vec::new() }
    }

    pub fn log(&self, req: &RequestInfo, status: u16, reason: &str) {
        if self.sinks.is_empty() {
            return;
        }
        let source = java_host_address(req.source);
        let xff = req.forwarded_for.clone().unwrap_or_else(|| source.clone());
        let line = self.format.format(&[
            Arg::Str(source),
            Arg::Str(xff),
            Arg::Str(req.method.clone()),
            Arg::Str(req.address.clone()),
            Arg::Str(req.version.to_owned()),
            Arg::Int(status.into()),
            Arg::Str(reason.to_owned()),
        ]);
        let level = if status < 500 { Level::Info } else { Level::Warning };
        self.sinks.iter().for_each(|s| s.log(level, &line));
    }
}

/// `InetAddress.getHostAddress`: IPv4-mapped addresses as IPv4, IPv6 uncompressed (`0:0:0:0:0:0:0:1`).
pub fn java_host_address(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => v6.segments().iter().map(|s| format!("{s:x}")).collect::<Vec<_>>().join(":"),
        },
    }
}

/// `Logger.file` with BlueMap's `LogFormatter`: `[yyyy-MM-dd HH:mm:ss][LEVEL] msg`. Writes on a background thread
/// so requests never wait on disk; flushes whenever the queue runs dry.
pub struct FileSink {
    tx: Mutex<Sender<String>>,
}

impl FileSink {
    /// `file_pattern` is `String.format`ted with the current local time as argument 1, like BlueMap does.
    pub fn open(file_pattern: &str, append: bool) -> Result<Self, WebError> {
        let name = JavaFormat::compile(file_pattern)?.format(&[Arg::Time(Local::now())]);
        let path = PathBuf::from(name);
        let io_err = |source| WebError::LogFile { path: path.clone(), source };
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(io_err)?;
        }
        let file =
            OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&path).map_err(io_err)?;
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::Builder::new()
            .name("bm-web-log".into())
            .spawn(move || write_loop(BufWriter::new(file), rx))
            .map_err(io_err)?;
        Ok(Self { tx: Mutex::new(tx) })
    }
}

fn write_loop(mut out: BufWriter<File>, rx: mpsc::Receiver<String>) {
    while let Ok(line) = rx.recv() {
        let _ = out.write_all(line.as_bytes());
        while let Ok(more) = rx.try_recv() {
            let _ = out.write_all(more.as_bytes());
        }
        let _ = out.flush();
    }
}

const LINE_SEP: &str = if cfg!(windows) { "\r\n" } else { "\n" };

impl LogSink for FileSink {
    fn log(&self, level: Level, message: &str) {
        let line = format!("[{}][{}] {message}{LINE_SEP}", Local::now().format("%Y-%m-%d %H:%M:%S"), level.name());
        if let Ok(tx) = self.tx.lock() {
            let _ = tx.send(line);
        }
    }
}

/// BlueMap's `-b` verbose console logger: `[HH:mm:ss LEVEL] msg`.
pub struct StdoutSink;

impl LogSink for StdoutSink {
    fn log(&self, level: Level, message: &str) {
        println!("[{} {}] {message}", Local::now().format("%H:%M:%S"), level.name());
    }
}
