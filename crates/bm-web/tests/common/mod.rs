//! A raw HTTP/1.1 client (exact targets, visible framing) and an in-process server runner.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bm_web::{Level, LogSink, WebApp, WebServer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// Body after de-chunking; still content-encoded.
    pub body: Vec<u8>,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().rev().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// Body with `Content-Encoding` undone (gzip, and the zstd we send for packed hires tiles).
    pub fn decoded(&self) -> Vec<u8> {
        match self.header("content-encoding") {
            Some("gzip") => gunzip(&self.body),
            Some("zstd") => bm_compress::Compression::Zstd.decompress(&self.body, 1 << 30).expect("valid zstd"),
            Some(other) => panic!("unexpected content-encoding {other}"),
            None => self.body.clone(),
        }
    }
}

pub fn gunzip(data: &[u8]) -> Vec<u8> {
    bm_compress::Compression::Gzip.decompress(data, 1 << 30).expect("valid gzip")
}

pub fn get(addr: SocketAddr, target: &str, headers: &[(&str, &str)]) -> Reply {
    request(addr, "GET", target, headers)
}

pub fn request(addr: SocketAddr, method: &str, target: &str, headers: &[(&str, &str)]) -> Reply {
    let mut s = TcpStream::connect(addr).expect("connect");
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut req = format!("{method} {target} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).unwrap();
    let raw = read_response(&mut s, method == "HEAD");
    parse(&raw, method == "HEAD")
}

/// Reads one response by its framing: Java ignores `Connection: close`, so EOF never comes.
fn read_response(s: &mut TcpStream, head: bool) -> Vec<u8> {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    let mut fill = |raw: &mut Vec<u8>| match s.read(&mut chunk) {
        Ok(0) | Err(_) => false,
        Ok(n) => {
            raw.extend_from_slice(&chunk[..n]);
            true
        }
    };
    let split = loop {
        if let Some(p) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break p + 4;
        }
        if !fill(&mut raw) {
            return raw;
        }
    };
    let reply = parse(&raw, true);
    if head || matches!(reply.status, 100..=199 | 204 | 304) {
        return raw;
    }
    let chunked = reply.header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked"));
    let length = reply.header("content-length").and_then(|v| v.parse::<usize>().ok());
    loop {
        let body = &raw[split..];
        let done = if chunked {
            body.ends_with(b"0\r\n\r\n") && dechunk_complete(body)
        } else {
            length.is_some_and(|l| body.len() >= l)
        };
        if done || !fill(&mut raw) {
            return raw;
        }
    }
}

fn dechunk_complete(mut data: &[u8]) -> bool {
    loop {
        let Some(eol) = data.windows(2).position(|w| w == b"\r\n") else { return false };
        let Ok(size) = usize::from_str_radix(String::from_utf8_lossy(&data[..eol]).trim(), 16) else { return false };
        if size == 0 {
            return true;
        }
        if data.len() < eol + 2 + size + 2 {
            return false;
        }
        data = &data[eol + 2 + size + 2..];
    }
}

pub fn parse(raw: &[u8], head: bool) -> Reply {
    // status 0 = the server closed the connection without answering
    let Some(split) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
        return Reply { status: 0, headers: Vec::new(), body: Vec::new() };
    };
    let head_text = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head_text.split("\r\n");
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers: Vec<(String, String)> =
        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect();
    let mut reply = Reply { status, headers, body: Vec::new() };
    let rest = &raw[split + 4..];
    reply.body = if head {
        Vec::new()
    } else if reply.header("transfer-encoding").is_some_and(|v| v.eq_ignore_ascii_case("chunked")) {
        dechunk(rest)
    } else {
        rest.to_vec()
    };
    reply
}

/// Chunked transfer decoding; stops at the terminating or at an incomplete chunk.
pub fn dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(eol) = data.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(String::from_utf8_lossy(&data[..eol]).trim(), 16).unwrap_or(0);
        if size == 0 || data.len() < eol + 2 + size + 2 {
            break;
        }
        out.extend_from_slice(&data[eol + 2..eol + 2 + size]);
        data = &data[eol + 2 + size + 2..];
    }
    out
}

/// A server on its own runtime and thread; stopped (gracefully) on drop.
pub struct Served {
    pub addr: SocketAddr,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Served {
    pub fn start(app: WebApp) -> Self {
        let (addr_tx, addr_rx) = std::sync::mpsc::channel();
        let (stop, stop_rx) = tokio::sync::oneshot::channel::<()>();
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
            rt.block_on(async move {
                let server = WebServer::bind("127.0.0.1", 0).await.unwrap();
                addr_tx.send(server.local_addr()).unwrap();
                server
                    .serve(app, async {
                        let _ = stop_rx.await;
                    })
                    .await
                    .unwrap();
            });
        });
        Self { addr: addr_rx.recv().unwrap(), stop: Some(stop), thread: Some(thread) }
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Collects access-log lines.
#[derive(Clone, Default)]
pub struct Capture(pub Arc<Mutex<Vec<(Level, String)>>>);

impl LogSink for Capture {
    fn log(&self, level: Level, message: &str) {
        self.0.lock().unwrap().push((level, message.to_owned()));
    }
}
