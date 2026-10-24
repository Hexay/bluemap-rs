"""Lowres PNG size vs lossless alternatives, per lod, with coverage (share of opaque pixels)."""
import collections
import io
import lzma
import multiprocessing as mp
import os
import sys

from PIL import Image


def one(path):
    data = open(path, "rb").read()
    im = Image.open(io.BytesIO(data))
    im.load()
    px = im.tobytes()
    out = {"n": 1, "png": len(data)}
    alpha = im.getchannel("A").tobytes() if "A" in im.mode else b"\xff"
    cover = sum(1 for a in alpha[::7] if a) / max(1, len(alpha[::7]))
    out["full"] = int(cover > 0.99)
    for name, kw in (("webp_ll", dict(format="WEBP", lossless=True, quality=100, method=6, exact=True)),):
        b = io.BytesIO()
        im.save(b, **kw)
        out[name] = b.tell()
        back = Image.open(io.BytesIO(b.getvalue())).convert(im.mode)
        out[name + "_ok"] = int(back.tobytes() == px)
    out["xz_raw"] = len(lzma.compress(px, preset=9 | lzma.PRESET_EXTREME))
    if out["full"]:
        out["png_full"] = out["png"]
        out["webp_full"] = out["webp_ll"]
    return path.replace("\\", "/").split("/tiles/")[1].split("/")[0], im.mode, im.size, out


def main():
    root = sys.argv[1]
    pngs = [os.path.join(d, f) for d, _, fs in os.walk(root) for f in fs if f.endswith(".png") and "/tiles/" in d.replace("\\", "/") + "/"]
    per_lod = collections.defaultdict(collections.Counter)
    modes = collections.Counter()
    with mp.Pool(4) as pool:
        for lod, mode, size, s in pool.imap_unordered(one, pngs, chunksize=2):
            per_lod[lod].update(s)
            modes[(mode, size)] += 1
    print("modes", dict(modes))
    print("lod  n   png_kB  webp_kB (ratio) xz_raw_kB | full n  png_kB  webp_kB  ok")
    for lod in sorted(per_lod):
        c = per_lod[lod]
        print(f"{lod:3s} {c['n']:4d} {c['png'] / 1e3:8.0f} {c['webp_ll'] / 1e3:8.0f} ({c['webp_ll'] / c['png']:.3f}) "
              f"{c['xz_raw'] / 1e3:8.0f} | {c['full']:4d} {c['png_full'] / 1e3:8.0f} {c['webp_full'] / 1e3:8.0f}"
              f"  {c['webp_ll_ok']}/{c['n']}")


if __name__ == "__main__":
    main()
