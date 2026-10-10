//! Transcoding for clients that can't take the stored coding. Results are cached by exact input, so hot items
//! (map `settings.json`, `textures.json` for identity clients) are encoded once per content; concurrent work is
//! bounded so decoded buffers can't pile up with the connection count.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Condvar, LazyLock, Mutex, MutexGuard};

use bm_compress::zstd_bulk;
use bm_storage::{Compression, MAX_DECODED, Stored, with_unpacked_hires};
use bytes::Bytes;

/// Room for a few hundred packed hires tiles (~6 KB in, ~70 KB out each) within [`MAX_BYTES`].
const MAX_ENTRIES: usize = 1024;
const MAX_BYTES: usize = 32 << 20;
const MAX_CACHED_OUTPUT: usize = MAX_BYTES / 4;
const SAMPLE: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Op {
    ToGzip(Compression),
    Decode(Compression),
    /// A packed hires tile to the given coding.
    Packed(Compression),
    /// The zstd frame of a packed hires tile's model body to the given coding.
    PackedBody(Compression),
}

struct Entry {
    input: Bytes,
    output: Bytes,
    used: u64,
}

struct Cache {
    /// By operation and input fingerprint.
    entries: HashMap<(Op, u64), Entry>,
    /// Input and output bytes of all entries.
    bytes: usize,
    tick: u64,
    /// Recently transcoded inputs: of stored items only a repeat is admitted, so a stream of distinct ones doesn't
    /// churn the cache.
    seen: [u64; SEEN],
}

const SEEN: usize = 64;
static CACHE: LazyLock<Mutex<Cache>> =
    LazyLock::new(|| Mutex::new(Cache { entries: HashMap::new(), bytes: 0, tick: 0, seen: [0; SEEN] }));
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

/// A packed hires tile (`MapStorage::read_hires_packed`) as PRBM in `coding`. Cached by the blob, so a hit skips
/// unpacking too; identity bodies are too large to be worth keeping.
pub(crate) fn packed(blob: &[u8], coding: Compression) -> Result<Bytes, bm_storage::Error> {
    match coding {
        Compression::None => Ok(bm_storage::unpack_hires(blob)?.into()),
        Compression::Zstd => cached(Op::Packed(coding), blob, || Ok(with_unpacked_hires(blob, zstd_bulk)??)),
        c => cached(Op::Packed(c), blob, || Ok(with_unpacked_hires(blob, |prbm| c.compress(prbm))??)),
    }
}

/// The model body of a packed hires tile in `coding`, for a client that unpacks it itself
/// (`bm_storage::packed_model_frame` of `blob` must be `Some`). The stored frame is zstd already.
pub(crate) fn packed_body(blob: Vec<u8>, coding: Compression) -> Result<Bytes, bm_compress::Error> {
    let blob = Bytes::from(blob);
    let frame = blob.slice_ref(bm_storage::packed_model_frame(&blob).unwrap_or_default());
    let body = || Compression::Zstd.decompress(&frame, MAX_DECODED);
    match coding {
        Compression::Zstd => Ok(frame.clone()),
        Compression::None => cached(Op::PackedBody(coding), &frame, body),
        c => cached(Op::PackedBody(c), &frame, || c.compress(&body()?)),
    }
}

fn cached<E>(op: Op, input: &[u8], work: impl FnOnce() -> Result<Vec<u8>, E>) -> Result<Bytes, E> {
    let fingerprint = fingerprint(input);
    if let Some(hit) = lookup(op, fingerprint, input) {
        return Ok(hit);
    }
    let output = Bytes::from({
        let _permit = GATE.acquire();
        work()?
    });
    // a packed tile costs milliseconds to redo and little to keep, so it is admitted on first sight
    let first_sight = matches!(op, Op::Packed(_) | Op::PackedBody(_));
    if output.len() <= MAX_CACHED_OUTPUT && (first_sight || seen_before(fingerprint)) {
        insert((op, fingerprint), Entry { input: Bytes::copy_from_slice(input), output: output.clone(), used: 0 });
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
        let e = cache.entries.get_mut(&(op, fingerprint))?;
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

fn insert(key: (Op, u64), mut entry: Entry) {
    let size = |e: &Entry| e.input.len() + e.output.len();
    let mut guard = lock();
    let cache = &mut *guard;
    cache.tick += 1;
    entry.used = cache.tick;
    cache.bytes += size(&entry);
    if let Some(replaced) = cache.entries.insert(key, entry) {
        cache.bytes -= size(&replaced);
    }
    while cache.entries.len() > MAX_ENTRIES || (cache.bytes > MAX_BYTES && cache.entries.len() > 1) {
        let oldest = *cache.entries.iter().min_by_key(|(_, e)| e.used).expect("non-empty").0;
        cache.bytes -= cache.entries.remove(&oldest).map_or(0, |e| size(&e));
    }
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
