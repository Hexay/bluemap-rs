"""Harness for the BMQ3 experiments: quad arrays of a tile, the shipped BMQ2 streams as baseline, and a parallel
runner that sizes variant stream sets as one zstd frame (shipped level-9 parameters, and level 19).

A variant is a top-level function `f(tile) -> {stream name: bytes}` whose result replaces/extends the baseline streams
(a value of None drops that stream). Sizes are only meaningful for invertible transforms; nothing here decodes.
"""
import collections
import glob
import lzma
import multiprocessing
import os
import re
import struct
import sys
import time

import brotli
import numpy as np
import zstandard as zstd

import bmq
import prbm

HERE = os.path.dirname(os.path.abspath(__file__))
REAL = os.path.join(HERE, "..", "..", "work", "exp", "real")
NAMES = ["pos", "pos_esc", "uv", "uv_esc", "ao", "normal_exc", "color", "color_exc", "light", "light_exc", "groups"]
SHIPPED = zstd.ZstdCompressionParameters.from_level(9, window_log=20, hash_log=18, chain_log=18)
WORKERS = 4


def paths(corpus, step=1):
    """'real' = the testbox sample under work/exp/real, 'fx' = the golden fixture webroots."""
    if corpus == "real":
        found = sorted(glob.glob(f"{REAL}/**/*.prbm.gz", recursive=True))
    else:
        found = [p for _, p in prbm.corpus()]
    return found[::step]


def rows(a):
    """[n, ...] -> [n] structured array, one comparable/hashable element per row (for np.unique)."""
    a = np.ascontiguousarray(a).reshape(len(a), -1)
    return a.view([("", a.dtype)] * a.shape[1]).reshape(-1)


_Z9 = zstd.ZstdCompressor(compression_params=SHIPPED)
# bodies stay under the 1 MiB window; the level's default 8 MiB window only costs memory per worker
_Z19 = zstd.ZstdCompressor(compression_params=zstd.ZstdCompressionParameters.from_level(19, window_log=21))


def z9(b):
    return len(_Z9.compress(b))


def z19(b):
    return len(_Z19.compress(b))


def join(streams):
    return b"".join(struct.pack("<I", len(b)) + b for b in streams.values())


def load(path):
    """-> tile dict, or None for empty / non-quad tiles (stored raw by BMQ2)."""
    gz, raw = prbm.load(path)
    t = prbm.parse(raw)
    if t["n"] == 0 or not bmq.is_quad_tile(t):
        return None
    Q = t["n"] // 6
    blob = bmq.encode(raw)
    tile = re.search(r"x(-?\d+)z(-?\d+)\.prbm", path.replace("\\", "/").split("/tiles/0/")[-1].replace("/", ""))
    return {
        "Q": Q,
        # default hires grid: 32-block tiles offset by 2
        "origin": (int(tile[1]) * 32 + 2, int(tile[2]) * 32 + 2),
        "gz": len(gz),
        "gp": blob[5],
        "gu": blob[6],
        "pos": t["position"].reshape(Q, 6, 3)[:, bmq.QV],
        "uv": t["uv"].reshape(Q, 6, 2)[:, bmq.QV],
        "ao": t["ao"].reshape(Q, 6)[:, bmq.QV],
        "color": t["color"].reshape(Q, 6, 3)[:, 0],
        "block": t["blocklight"].reshape(Q, 6)[:, 0],
        "sun": t["sunlight"].reshape(Q, 6)[:, 0],
        "groups": np.stack([t["groups"][:, 0], t["groups"][:, 2] // 6], 1),
        "base": dict(zip(NAMES, bmq.unstreams(blob, 11))),
    }


def baseline(_tile):
    return {}


def body_streams(tile, module, name):
    mod = sys.modules.get(module) or __import__(module)
    streams = dict(tile["base"])
    streams.update(getattr(mod, name)(tile))
    return {k: v for k, v in streams.items() if v is not None}


LZMA = [{"id": lzma.FILTER_LZMA2, "preset": 9 | lzma.PRESET_EXTREME, "dict_size": 1 << 22}]
CODECS = {
    "zstd-9 shipped": z9,
    "zstd-12": lambda b: len(zstd.ZstdCompressor(level=12).compress(b)),
    "zstd-15": lambda b: len(zstd.ZstdCompressor(level=15).compress(b)),
    "zstd-19": z19,
    "zstd-22": lambda b: len(zstd.ZstdCompressor(level=22).compress(b)),
    "brotli-11": lambda b: len(brotli.compress(b, quality=11, lgwin=22)),
    "xz-9e": lambda b: len(lzma.compress(b, format=lzma.FORMAT_RAW, filters=LZMA)),
}


def sweep(module, name, corpus, step):
    """Whole-body size and single-thread encode speed of one variant under every codec in CODECS."""
    size, secs, raw = dict.fromkeys(CODECS, 0), dict.fromkeys(CODECS, 0.0), 0
    for p in paths(corpus, step):
        tile = load(p)
        if tile is None:
            continue
        body = join(body_streams(tile, module, name))
        raw += len(body)
        for codec, f in CODECS.items():
            start = time.perf_counter()
            size[codec] += f(body)
            secs[codec] += time.perf_counter() - start
    print(f"{name} on every {step}th {corpus} tile, body {raw / 1e6:.1f} MB")
    for codec in CODECS:
        print(f"   {codec:15s} {size[codec] / 1e6:7.3f} MB  {size[codec] / size['zstd-9 shipped']:.3f}"
              f"  encode {raw / 1e6 / secs[codec]:6.1f} MB/s")


def _work(job):
    path, module, names = job
    tile = load(path)
    if tile is None:
        return None
    out = {"Q": tile["Q"], "gz": tile["gz"]}
    for name in names:
        streams = body_streams(tile, module, name)
        body = join(streams)
        out[name] = (z9(body), z19(body), {k: (len(v), z19(v)) for k, v in streams.items()})
    return out


def run(module, names, corpus="real", step=1, detail=()):
    """Sizes every variant in `names` (functions of `module`) over the corpus and prints totals relative to the first."""
    files = paths(corpus, step)
    start = time.time()
    with multiprocessing.Pool(WORKERS) as pool:
        results = [r for r in pool.imap_unordered(_work, [(p, module, names) for p in files], chunksize=4) if r]
    quads = sum(r["Q"] for r in results)
    gz = sum(r["gz"] for r in results)
    print(f"{corpus}: {len(results)} quad tiles of {len(files)}, {quads / 1e6:.2f} M quads, stored gz {gz / 1e6:.1f} MB"
          f" ({time.time() - start:.0f} s)")
    ref = None
    for name in names:
        a, b = sum(r[name][0] for r in results), sum(r[name][1] for r in results)
        ref = ref or (a, b)
        print(f"{name:22s} z9 {a / 1e6:8.3f} MB {a / ref[0]:6.3f}  {8 * a / quads:5.2f} b/quad  gz/z9 {gz / a:5.2f}x |"
              f" z19 {b / 1e6:8.3f} MB {b / ref[1]:6.3f}")
    for name in detail:
        raw, comp = collections.Counter(), collections.Counter()
        for r in results:
            for k, (n, z) in r[name][2].items():
                raw[k] += n
                comp[k] += z
        total = sum(comp.values())
        print(f"-- {name}: streams compressed alone (z19)")
        for k in raw:
            print(f"   {k:14s} raw {raw[k] / 1e6:8.2f} MB  z19 {comp[k] / 1e6:7.3f} MB  {100 * comp[k] / total:5.1f}%"
                  f"  {8 * comp[k] / quads:5.2f} b/quad")
