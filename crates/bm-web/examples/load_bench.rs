//! Closed-loop HTTP/1.1 keep-alive load generator for `serve_bench`; prints one JSON summary line.
//!
//! `load_bench <addr> <urls-file> [--conns 32] [--secs 10] [--header "Name: value"]… [--sse N] [--revalidate]`
//!
//! Each connection walks the URL list from its own offset. `--revalidate` sends each URL's `ETag` (learned in one
//! untimed pass) as `If-None-Match`, like a browser reload. `--sse N` instead holds N `live/sse` streams open (the
//! first URL) for `--secs` and counts received bytes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

#[derive(Default)]
struct ConnStats {
    lat_us: Vec<u32>,
    bytes: u64,
    status: [u64; 6],
    errors: u64,
}

struct Head {
    status: u16,
    body: u64,
    etag: Option<String>,
}

fn read_response(r: &mut BufReader<TcpStream>, line: &mut String) -> std::io::Result<Head> {
    line.clear();
    r.read_line(line)?;
    let status: u16 = line.split(' ').nth(1).and_then(|s| s.parse().ok()).ok_or(std::io::ErrorKind::InvalidData)?;
    let (mut len, mut chunked, mut etag) = (None, false, None);
    loop {
        line.clear();
        if r.read_line(line)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            let (k, v) = (k.trim(), v.trim());
            if k.eq_ignore_ascii_case("content-length") {
                len = v.parse::<u64>().ok();
            } else if k.eq_ignore_ascii_case("transfer-encoding") && v.eq_ignore_ascii_case("chunked") {
                chunked = true;
            } else if k.eq_ignore_ascii_case("etag") {
                etag = Some(v.to_owned());
            }
        }
    }
    let mut body = 0;
    if matches!(status, 204 | 304) {
    } else if chunked {
        loop {
            line.clear();
            r.read_line(line)?;
            let n = u64::from_str_radix(line.trim(), 16).map_err(|_| std::io::ErrorKind::InvalidData)?;
            std::io::copy(&mut r.by_ref().take(n + 2), &mut std::io::sink())?;
            body += n;
            if n == 0 {
                break;
            }
        }
    } else if let Some(n) = len {
        body = std::io::copy(&mut r.by_ref().take(n), &mut std::io::sink())?;
    }
    Ok(Head { status, body, etag })
}

fn connect(addr: &str) -> std::io::Result<(TcpStream, BufReader<TcpStream>)> {
    let s = TcpStream::connect(addr)?;
    s.set_nodelay(true)?;
    let r = BufReader::with_capacity(256 * 1024, s.try_clone()?);
    Ok((s, r))
}

/// `etags`: per URL the last `ETag` seen, sent back as `If-None-Match` (a browser reload); filled before timing.
fn worker(
    addr: &str,
    urls: &[String],
    offset: usize,
    headers: &str,
    etags: Option<&[Option<String>]>,
    stop: &AtomicBool,
) -> ConnStats {
    let mut st = ConnStats::default();
    let Ok((mut w, mut r)) = connect(addr) else {
        st.errors += 1;
        return st;
    };
    let mut line = String::new();
    let mut i = offset;
    while !stop.load(Relaxed) {
        let k = i % urls.len();
        let url = &urls[k];
        i += 1;
        let inm = match etags.and_then(|e| e[k].as_deref()) {
            Some(tag) => format!("If-None-Match: {tag}\r\n"),
            None => String::new(),
        };
        let req = format!("GET /{url} HTTP/1.1\r\nHost: {addr}\r\n{headers}{inm}\r\n");
        let t = Instant::now();
        let res = w.write_all(req.as_bytes()).and_then(|()| read_response(&mut r, &mut line));
        match res {
            Ok(head) => {
                st.lat_us.push(t.elapsed().as_micros().min(u32::MAX as u128) as u32);
                st.bytes += head.body;
                st.status[(head.status / 100) as usize % 6] += 1;
            }
            Err(_) => {
                st.errors += 1;
                match connect(addr) {
                    Ok(c) => (w, r) = c,
                    Err(_) => break,
                }
            }
        }
    }
    st
}

/// One untimed pass over `urls` collecting each `ETag`.
fn learn_etags(addr: &str, urls: &[String], headers: &str) -> Vec<Option<String>> {
    let (mut w, mut r) = connect(addr).expect("connect");
    let mut line = String::new();
    urls.iter()
        .map(|url| {
            write!(w, "GET /{url} HTTP/1.1\r\nHost: {addr}\r\n{headers}\r\n").expect("send");
            read_response(&mut r, &mut line).expect("response").etag
        })
        .collect()
}

fn sse(addr: &str, url: &str, n: usize, secs: u64) {
    let total = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let handles: Vec<_> = (0..n)
        .map(|_| {
            let (addr, url, total, stop) = (addr.to_owned(), url.to_owned(), total.clone(), stop.clone());
            std::thread::spawn(move || {
                let Ok((mut w, mut r)) = connect(&addr) else { return };
                let _ = w.set_read_timeout(Some(Duration::from_millis(500)));
                let _ = write!(w, "GET /{url} HTTP/1.1\r\nHost: {addr}\r\n\r\n");
                let mut buf = [0u8; 16 * 1024];
                while !stop.load(Relaxed) {
                    if let Ok(k) = r.read(&mut buf) {
                        if k == 0 {
                            break;
                        }
                        total.fetch_add(k as u64, Relaxed);
                    }
                }
            })
        })
        .collect();
    std::thread::sleep(Duration::from_secs(secs));
    stop.store(true, Relaxed);
    handles.into_iter().for_each(|h| drop(h.join()));
    println!("{{\"sse_clients\":{n},\"secs\":{secs},\"bytes\":{}}}", total.load(Relaxed));
}

fn main() {
    let mut args = std::env::args().skip(1);
    let addr = args.next().expect("addr");
    let urls: Vec<String> = std::fs::read_to_string(args.next().expect("urls file"))
        .expect("read urls")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.trim_start_matches('/').to_owned())
        .collect();
    let (mut conns, mut secs, mut headers, mut sse_n, mut revalidate) = (32usize, 10u64, String::new(), 0usize, false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--conns" => conns = args.next().unwrap().parse().unwrap(),
            "--secs" => secs = args.next().unwrap().parse().unwrap(),
            "--header" => headers.push_str(&format!("{}\r\n", args.next().unwrap())),
            "--sse" => sse_n = args.next().unwrap().parse().unwrap(),
            "--revalidate" => revalidate = true,
            other => panic!("unknown arg {other}"),
        }
    }
    if sse_n > 0 {
        return sse(&addr, &urls[0], sse_n, secs);
    }
    let etags = revalidate.then(|| Arc::new(learn_etags(&addr, &urls, &headers)));
    let urls = Arc::new(urls);
    let stop = Arc::new(AtomicBool::new(false));
    let started = Instant::now();
    let handles: Vec<_> = (0..conns)
        .map(|c| {
            let (addr, urls, headers, stop) = (addr.clone(), urls.clone(), headers.clone(), stop.clone());
            let etags = etags.clone();
            let offset = c * urls.len() / conns.max(1);
            std::thread::spawn(move || worker(&addr, &urls, offset, &headers, etags.as_deref().map(|e| &e[..]), &stop))
        })
        .collect();
    std::thread::sleep(Duration::from_secs(secs));
    stop.store(true, Relaxed);
    let mut all = ConnStats::default();
    for h in handles {
        let s = h.join().unwrap();
        all.lat_us.extend(s.lat_us);
        all.bytes += s.bytes;
        all.errors += s.errors;
        (0..6).for_each(|i| all.status[i] += s.status[i]);
    }
    let elapsed = started.elapsed().as_secs_f64();
    all.lat_us.sort_unstable();
    let n = all.lat_us.len();
    let pct = |p: f64| if n == 0 { 0 } else { all.lat_us[((n as f64 * p) as usize).min(n - 1)] };
    println!(
        "{{\"requests\":{n},\"secs\":{elapsed:.3},\"rps\":{:.1},\"body_bytes\":{},\"mb_per_s\":{:.2},\
         \"p50_us\":{},\"p90_us\":{},\"p99_us\":{},\"max_us\":{},\"s2xx\":{},\"s3xx\":{},\"s4xx\":{},\"s5xx\":{},\"errors\":{}}}",
        n as f64 / elapsed,
        all.bytes,
        all.bytes as f64 / elapsed / 1e6,
        pct(0.5),
        pct(0.9),
        pct(0.99),
        pct(1.0),
        all.status[2],
        all.status[3],
        all.status[4],
        all.status[5],
        all.errors
    );
}
