"""Per-category file inventory (count, bytes, 4K clusters, dirs), hires vertex-count histogram, rstate and webapp asset compressibility."""
import collections
import glob
import gzip
import os
import sys
import zlib

import brotli
import zstandard

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import prbm  # noqa: E402

ROOT = prbm.ROOT


def category(rel):
    if "/tiles/0/" in rel:
        return "hires"
    if "/tiles/" in rel:
        return "lowres" + rel.split("/tiles/")[1].split("/")[0]
    if "/rstate/regions/" in rel:
        return "rstate-region"
    if rel.endswith(".tiles.dat"):
        return "rstate-tiles"
    if rel.endswith(".chunks.dat"):
        return "rstate-chunks"
    return os.path.basename(rel)


def hires_vertices(path):
    with open(path, "rb") as f:
        head = zlib.decompressobj(31).decompress(f.read(4096), 8)  # dynamic-Huffman header can exceed 64 B
    return int.from_bytes(head[2:5], "little")


def comp(data):
    return (len(data), len(gzip.compress(data, 6, mtime=0)),
            len(zstandard.ZstdCompressor(level=19).compress(data)), len(brotli.compress(data, quality=11)))


def main():
    cats = collections.defaultdict(collections.Counter)
    dirs = 0
    for mapdir in sorted(glob.glob(f"{ROOT}/*/web/maps/*")):
        for d, sub, files in os.walk(mapdir):
            dirs += 1
            for fn in files:
                p = os.path.join(d, fn).replace("\\", "/")
                s = os.path.getsize(p)
                c = cats[category(p)]
                c["n"] += 1
                c["bytes"] += s
                c["disk"] += prbm.cluster(s)
                if s < 4096:
                    c["sub4k"] += 1
    print("category            n      bytes    4K-disk  files<4K")
    tot = collections.Counter()
    for k in sorted(cats):
        c = cats[k]
        tot.update(c)
        print(f"{k:16s} {c['n']:6d} {c['bytes']:10d} {c['disk']:10d} {c['sub4k']:6d}")
    print(f"{'TOTAL':16s} {tot['n']:6d} {tot['bytes']:10d} {tot['disk']:10d} {tot['sub4k']:6d}   dirs={dirs}")

    buckets = [0, 600, 6000, 60000, 600000, 1 << 30]
    hist = collections.Counter()
    hist_b = collections.Counter()
    for _, p in prbm.corpus():
        v = hires_vertices(p)
        b = next(i for i, lim in enumerate(buckets) if v <= lim)
        hist[b] += 1
        hist_b[b] += os.path.getsize(p)
    print("\nhires vertex-count buckets (<=limit): n, gz bytes")
    for i, lim in enumerate(buckets):
        print(f"  <= {lim:>10d}: {hist[i]:5d} {hist_b[i]:10d}")

    rs = collections.Counter()
    for p in glob.glob(f"{ROOT}/*/web/maps/*/rstate/**/*.dat", recursive=True):
        raw = gzip.decompress(open(p, "rb").read())
        rs["files"] += 1
        rs["gz"] += os.path.getsize(p)
        rs["raw"] += len(raw)
        rs["zstd19"] += len(zstandard.ZstdCompressor(level=19).compress(raw))
    print("\nrstate", dict(rs))

    web = glob.glob(f"{ROOT}/structures/web/assets/*") + glob.glob(f"{ROOT}/structures/web/index.html")
    print("\nwebapp asset            raw      gz6    zstd19    br11")
    for p in sorted(web):
        r, g, z, b = comp(open(p, "rb").read())
        print(f"{os.path.basename(p):26s} {r:8d} {g:8d} {z:8d} {b:8d}")
    lang = glob.glob(f"{ROOT}/structures/web/lang/*")
    print("lang files", len(lang), sum(os.path.getsize(p) for p in lang))


if __name__ == "__main__":
    main()
