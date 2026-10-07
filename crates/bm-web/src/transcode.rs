//! Transcoding for clients that can't take the stored coding. Results are cached by exact input, so hot items
//! (map `settings.json`, `textures.json` for identity clients) are encoded once per content; concurrent work is
//! bounded so decoded buffers can't pile up with the connection count.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Condvar, LazyLock, Mutex, MutexGuard};

use bm_storage::{Compression, MAX_DECODED, Stored};
use bytes::Bytes;

const MAX_ENTRIES: usize = 16;
const MAX_BYTES: usize = 32 << 20;
const MAX_CACHED_OUTPUT: usize = MAX_BYTES / 4;
const SAMPLE: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    ToGzip(Compression),
    Decode(Compression),
}

struct Entry {
    op: Op,
    fingerprint: u64,
    input: Bytes,
    output: Bytes,
    used: u64,
}

struct Cache {
    entries: Vec<Entry>,
    tick: u64,
    /// Recently transcoded inputs: only a repeat is admitted, so a stream of distinct tiles doesn't churn the cache.
    seen: [u64; SEEN],
}

const SEEN: usize = 64;
static CACHE: Mutex<Cache> = Mutex::new(Cache { entries: Vec::new(), tick: 0, seen: [0; SEEN] });
static GATE: LazyLock<Gate> = LazyLock::new(|| Gate {
    busy: Mutex::new(0),
    freed: Condvar::new(),
    limit: std::thread::available_parallelism().map_or(4, usize::from),
});

/// `stored` as gzip bytes.
pub(crate) fn to_gzip(stored: &Stored) -> Result<Bytes, bm_compress::Error> {
    let c = stored.compression;
    cached(Op::ToGzip(c), &stored.data, || match c {
        Compression::None => Compression::Gzip.compress(&stored.data),
        c => Compression::Gzip.compress(&c.decompress(&stored.data, MAX_DECODED)?),
    })
}

/// `stored` decoded.
pub(crate) fn decode(stored: &Stored) -> Result<Bytes, bm_compress::Error> {
    let c = stored.compression;
    cached(Op::Decode(c), &stored.data, || c.decompress(&stored.data, MAX_DECODED))
}

fn cached(
    op: Op,
    input: &[u8],
    work: impl FnOnce() -> Result<Vec<u8>, bm_compress::Error>,
) -> Result<Bytes, bm_compress::Error> {
    let fingerprint = fingerprint(input);
    if let Some(hit) = lookup(op, fingerprint, input) {
        return Ok(hit);
    }
    let output = Bytes::from({
        let _permit = GATE.acquire();
        work()?
    });
    if output.len() <= MAX_CACHED_OUTPUT && seen_before(fingerprint) {
        insert(Entry { op, fingerprint, input: Bytes::copy_from_slice(input), output: output.clone(), used: 0 });
    }
    Ok(output)
}

/// Length plus both ends: a gzip trailer is CRC32 + size, so this separates real items; hits are verified in full.
fn fingerprint(input: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    input.len().hash(&mut h);
    input[..input.len().min(SAMPLE)].hash(&mut h);
    input[input.len().saturating_sub(SAMPLE)..].hash(&mut h);
    h.finish()
}

fn lock() -> MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

fn lookup(op: Op, fingerprint: u64, input: &[u8]) -> Option<Bytes> {
    let (stored_input, output) = {
        let mut cache = lock();
        cache.tick += 1;
        let tick = cache.tick;
        let e = cache.entries.iter_mut().find(|e| e.op == op && e.fingerprint == fingerprint)?;
        e.used = tick;
        (e.input.clone(), e.output.clone())
    };
    (stored_input == input).then_some(output)
}

fn seen_before(fingerprint: u64) -> bool {
    let mut cache = lock();
    if cache.seen.contains(&fingerprint) {
        return true;
    }
    let slot = (cache.tick as usize) % SEEN;
    cache.seen[slot] = fingerprint;
    false
}

fn insert(mut entry: Entry) {
    let mut cache = lock();
    cache.tick += 1;
    entry.used = cache.tick;
    cache.entries.retain(|e| !(e.op == entry.op && e.fingerprint == entry.fingerprint));
    let size = |e: &Entry| e.input.len() + e.output.len();
    let mut total: usize = cache.entries.iter().map(size).sum::<usize>() + size(&entry);
    while cache.entries.len() >= MAX_ENTRIES || (total > MAX_BYTES && !cache.entries.is_empty()) {
        let oldest = (0..cache.entries.len()).min_by_key(|&i| cache.entries[i].used).expect("non-empty");
        total -= size(&cache.entries.swap_remove(oldest));
    }
    cache.entries.push(entry);
}

/// A counting semaphore for blocking-pool threads.
struct Gate {
    busy: Mutex<usize>,
    freed: Condvar,
    limit: usize,
}

struct Permit<'a>(&'a Gate);

impl Gate {
    fn acquire(&self) -> Permit<'_> {
        let mut busy = self.busy.lock().unwrap_or_else(|e| e.into_inner());
        while *busy >= self.limit {
            busy = self.freed.wait(busy).unwrap_or_else(|e| e.into_inner());
        }
        *busy += 1;
        Permit(self)
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        *self.0.busy.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
        self.0.freed.notify_one();
    }
}
