"""Per-blob compressed sizes for gzip/zstd/brotli, plus zstd with a dictionary trained on a held-out half.
Blobs are produced inside workers by blob_fn(item) so the corpus never sits in memory at once.
brotli -11 runs at ~8 us/byte on PRBM, so it is measured only on every BROTLI_EVERY-th item (key prefix "brsub_")."""
import gzip
import random

import brotli
import zstandard as zstd

DICT_SIZES = (16 * 1024, 112 * 1024)
TRAIN_SAMPLES = 150
WORKERS = 8  # ~2 GB free RAM on the dev box
BROTLI_EVERY = 4


def sizes(blob, with_brotli):
    s = {
        "raw": len(blob),
        "gzip6": len(gzip.compress(blob, 6, mtime=0)),
        "gzip9": len(gzip.compress(blob, 9, mtime=0)),
        "zstd19": len(zstd.ZstdCompressor(level=19).compress(blob)),
    }
    if with_brotli:
        s.update(brsub_tiles=1, brsub_raw=len(blob), brsub_brotli11=len(brotli.compress(blob, quality=11)),
                 brsub_zstd19=s["zstd19"], brsub_gzip6=s["gzip6"])
    return s


def _plain(args):
    blob_fn, item, idx = args
    return sizes(blob_fn(item), idx % BROTLI_EVERY == 0)


def _with_dicts(args):
    blob_fn, item, dicts = args
    blob = blob_fn(item)
    return {f"zstd19_dict{DICT_SIZES[i] // 1024}k": len(zstd.ZstdCompressor(
        level=19, dict_data=zstd.ZstdCompressionDict(d)).compress(blob)) for i, d in enumerate(dicts)}


def split(n):
    """Deterministic train/test split of indices: even = train, odd = test."""
    return list(range(0, n, 2)), list(range(1, n, 2))


def train(blobs):
    blobs = [b for b in blobs if b]
    return [zstd.train_dictionary(s, blobs, level=19).as_bytes() for s in DICT_SIZES]


def measure(items, blob_fn, pool):
    """-> (per-item codec size dicts; dict variants only on the test half, actual dict byte sizes)."""
    out = pool.map(_plain, [(blob_fn, it, i) for i, it in enumerate(items)], chunksize=2)
    tr, te = split(len(items))
    pick = random.Random(2).sample(tr, min(TRAIN_SAMPLES, len(tr)))
    dicts = train(pool.map(blob_fn, [items[i] for i in pick], chunksize=2))
    for i, d in zip(te, pool.map(_with_dicts, [(blob_fn, items[i], dicts) for i in te], chunksize=2)):
        out[i].update(d)
    return out, [len(d) for d in dicts]
