"""Lowres PNG encoder settings (IHDR, zlib level, row filters), coverage, and same-format re-encode headroom."""
import collections
import io
import multiprocessing as mp
import os
import struct
import sys
import zlib

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import prbm  # noqa: E402
from codec_sizes import WORKERS  # noqa: E402


def chunks(data):
    i = 8
    while i < len(data):
        n, t = struct.unpack(">I4s", data[i:i + 8])
        yield t, data[i + 8:i + 8 + n]
        i += 12 + n


def png_bytes(arr, **kw):
    b = io.BytesIO()
    Image.fromarray(arr, "RGBA").save(b, format="PNG", **kw)
    return b.tell()


def one(path):
    data = open(path, "rb").read()
    cs = list(chunks(data))
    ihdr = cs[0][1]
    w, h, depth, ctype, _, _, interlace = struct.unpack(">IIBBBBB", ihdr)
    idat = b"".join(d for t, d in cs if t == b"IDAT")
    flevel = idat[1] >> 6
    raw = zlib.decompress(idat)
    stride = w * 4 + 1
    filters = collections.Counter(raw[r * stride] for r in range(h))
    a = np.asarray(Image.open(io.BytesIO(data)).convert("RGBA")).copy()
    top = a[: h // 2]
    opaque = float((top[..., 3] > 0).mean())
    hidden_rgb = int(((top[..., 3] == 0) & (top[..., :3].any(-1))).sum())
    out = collections.Counter(n=1, png=len(data), disk=prbm.cluster(len(data)),
                              chunks_other=sum(len(d) + 12 for t, d in cs if t not in (b"IDAT", b"IHDR", b"IEND")))
    out["opt"] = png_bytes(a, optimize=True)
    z = a.copy()
    t = z[: h // 2]
    t[t[..., 3] == 0] = 0
    out["opt_zero_hidden"] = png_bytes(z, optimize=True)
    if opaque == 0:
        out["empty"] += 1
        out["empty_bytes"] += len(data)
        out["empty_disk"] += prbm.cluster(len(data))
    elif opaque < 0.05:
        out["near_empty"] += 1
    out["opaque_sum"] = opaque
    out["hidden_rgb_px"] = hidden_rgb
    meta = a[h // 2:]
    out["meta_alpha_not255"] = int((meta[..., 3] != 255).sum())
    lod = path.split("/tiles/")[1].split("/")[0]
    return lod, (w, h, depth, ctype, interlace, flevel), filters, out


def main():
    per = collections.defaultdict(collections.Counter)
    fmt = collections.Counter()
    filt = collections.Counter()
    with mp.Pool(WORKERS) as pool:
        for lod, f, fl, o in pool.imap_unordered(one, prbm.lowres_pngs(), chunksize=4):
            per[lod].update(o)
            fmt[f] += 1
            filt.update(fl)
    print("IHDR (w,h,depth,ctype,interlace,zlib FLEVEL):", dict(fmt))
    print("row filter types:", dict(filt))
    print("lod n png disk opt opt+zeroHiddenRGB empty near_empty(<5%) mean_opaque hidden_rgb_px meta_alpha!=255 other_chunks")
    for lod in sorted(per):
        c = per[lod]
        print(lod, c["n"], c["png"], c["disk"], c["opt"], c["opt_zero_hidden"], c["empty"], c["near_empty"],
              round(c["opaque_sum"] / c["n"], 3), c["hidden_rgb_px"], c["meta_alpha_not255"], c["chunks_other"],
              "empty_bytes", c["empty_bytes"], "empty_disk", c["empty_disk"])


if __name__ == "__main__":
    main()
