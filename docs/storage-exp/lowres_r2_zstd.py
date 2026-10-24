"""Lowres PNG vs zstd-19 on pixels: raw RGBA, channel planes, planes after a left/up/paeth-like predictor."""
import io
import os
import sys

import numpy as np
import zstandard
from PIL import Image

Z = zstandard.ZstdCompressor(level=19)


def pred(a, kind):
    a = a.astype(np.int16)
    left = np.zeros_like(a); left[:, 1:] = a[:, :-1]
    up = np.zeros_like(a); up[1:, :] = a[:-1, :]
    if kind == "left":
        p = left
    elif kind == "up":
        p = up
    else:  # MED / LOCO-I
        ul = np.zeros_like(a); ul[1:, 1:] = a[:-1, :-1]
        mx, mn = np.maximum(left, up), np.minimum(left, up)
        p = np.where(ul >= mx, mn, np.where(ul <= mn, mx, left + up - ul))
    return ((a - p) & 0xFF).astype(np.uint8)


def main():
    root = sys.argv[1]
    tot = {}
    for d, _, fs in os.walk(root):
        for f in fs:
            if not f.endswith(".png") or "/1/" not in d.replace("\\", "/") + "/":
                continue
            data = open(os.path.join(d, f), "rb").read()
            px = np.asarray(Image.open(io.BytesIO(data)).convert("RGBA"))
            planes = [px[:, :, c] for c in range(4)]
            r = {"png": len(data), "raw": len(Z.compress(px.tobytes())),
                 "planes": len(Z.compress(b"".join(p.tobytes() for p in planes)))}
            for k in ("left", "up", "med"):
                r[k] = len(Z.compress(b"".join(pred(p, k).tobytes() for p in planes)))
            for k, v in r.items():
                tot[k] = tot.get(k, 0) + v
    print({k: f"{v / 1e3:.0f} kB ({v / tot['png']:.3f})" for k, v in tot.items()})


if __name__ == "__main__":
    main()
