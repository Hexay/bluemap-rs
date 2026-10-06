"""Lowres PNG sizes vs PIL max-zlib PNG and WebP-lossless (exact=True keeps RGB under alpha 0); pixel-verified."""
import collections
import io
import multiprocessing as mp
import os
import sys

from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import prbm  # noqa: E402
from codec_sizes import WORKERS  # noqa: E402


def one(path):
    data = open(path, "rb").read()
    im = Image.open(io.BytesIO(data))
    im.load()
    px = im.tobytes()
    out = {"n": 1, "png": len(data), "disk4k": prbm.cluster(len(data))}
    for name, kw in (("png_z9", dict(format="PNG", optimize=True)),
                     ("webp_ll", dict(format="WEBP", lossless=True, quality=100, method=6, exact=True))):
        b = io.BytesIO()
        im.save(b, **kw)
        out[name] = b.tell()
        back = Image.open(io.BytesIO(b.getvalue())).convert(im.mode)
        out[name + "_ok"] = int(back.tobytes() == px)
    return path.split("/tiles/")[1].split("/")[0], im.mode, out


def main():
    pngs = prbm.lowres_pngs()
    per_lod = collections.defaultdict(collections.Counter)
    modes = collections.Counter()
    with mp.Pool(WORKERS) as pool:
        for lod, mode, s in pool.imap_unordered(one, pngs, chunksize=4):
            per_lod[lod].update(s)
            modes[mode] += 1
    print("modes", dict(modes))
    print("lod  n     png_kB  disk4k_kB  png_z9_kB  webp_ll_kB  ok(z9,webp)")
    for lod in sorted(per_lod):
        c = per_lod[lod]
        print(f"{lod:3s} {c['n']:4d} {c['png'] / 1e3:9.0f} {c['disk4k'] / 1e3:9.0f} {c['png_z9'] / 1e3:9.0f} "
              f"{c['webp_ll'] / 1e3:10.0f}   {c['png_z9_ok']},{c['webp_ll_ok']}")


if __name__ == "__main__":
    main()
