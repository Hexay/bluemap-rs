"""Same-format PNG headroom: re-deflate lowres IDAT at zlib 9 with each fixed row filter (0-4) and keep the best (oxipng-lite)."""
import collections
import multiprocessing as mp
import os
import sys
import zlib

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import prbm  # noqa: E402
from codec_sizes import WORKERS  # noqa: E402

BPP = 4
PNG_OVERHEAD = 8 + 25 + 12 + 12  # signature, IHDR, IDAT header/crc, IEND


def filtered(a, ftype):
    x = a.astype(np.int16)
    left = np.zeros_like(x)
    left[:, BPP:] = x[:, :-BPP]
    up = np.zeros_like(x)
    up[1:] = x[:-1]
    if ftype == 0:
        f = x
    elif ftype == 1:
        f = x - left
    elif ftype == 2:
        f = x - up
    elif ftype == 3:
        f = x - (left + up) // 2
    else:
        ul = np.zeros_like(x)
        ul[1:, BPP:] = x[:-1, :-BPP]
        p = left + up - ul
        pa, pb, pc = abs(p - left), abs(p - up), abs(p - ul)
        pred = np.where((pa <= pb) & (pa <= pc), left, np.where(pb <= pc, up, ul))
        f = x - pred
    rows = (f & 0xFF).astype(np.uint8)
    return np.hstack([np.full((a.shape[0], 1), ftype, np.uint8), rows]).tobytes()


def one(path):
    im =np.asarray(Image.open(path).convert("RGBA"))
    a = im.reshape(im.shape[0], -1)
    sizes = {f: len(zlib.compress(filtered(a, f), 9)) + PNG_OVERHEAD for f in range(5)}
    best = min(sizes, key=sizes.get)
    lod = path.split("/tiles/")[1].split("/")[0]
    return lod, best, collections.Counter(n=1, png=os.path.getsize(path), f0_z9=sizes[0], best=sizes[best])


def main():
    per = collections.defaultdict(collections.Counter)
    best = collections.Counter()
    with mp.Pool(WORKERS) as pool:
        for lod, b, c in pool.imap_unordered(one, prbm.lowres_pngs(), chunksize=4):
            per[lod].update(c)
            best[b] += 1
    print("best filter histogram:", dict(best))
    for lod in sorted(per):
        c = per[lod]
        print(f"lod{lod} n={c['n']} stored={c['png']} none+z9={c['f0_z9']} ({c['f0_z9'] / c['png']:.3f}) "
              f"best-filter+z9={c['best']} ({c['best'] / c['png']:.3f})")


if __name__ == "__main__":
    main()
