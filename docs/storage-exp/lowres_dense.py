"""Re-encode headroom on the densest lowres tiles (sparse fixtures understate it): PIL optimize, WebP-lossless, colour-half/meta-half split."""
import io
import os
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import prbm  # noqa: E402

TOP = 12


def enc(arr, **kw):
    b = io.BytesIO()
    Image.fromarray(arr, "RGBA").save(b, **kw)
    return b.tell()


def main():
    rows = []
    for p in prbm.lowres_pngs():
        a = np.asarray(Image.open(p).convert("RGBA"))
        rows.append((float((a[: a.shape[0] // 2, :, 3] > 0).mean()), p, a))
    rows.sort(key=lambda r: -r[0])
    tot = np.zeros(6)
    print("opaque  png  opt  webp_ll  colour_half_opt  meta_half_opt  path")
    for op, p, a in rows[:TOP]:
        h = a.shape[0] // 2
        s = np.array([os.path.getsize(p), enc(a, format="PNG", optimize=True),
                      enc(a, format="WEBP", lossless=True, quality=100, method=6, exact=True),
                      enc(np.ascontiguousarray(a[:h]), format="PNG", optimize=True),
                      enc(np.ascontiguousarray(a[h:]), format="PNG", optimize=True), 1])
        tot += s
        print(f"{op:.2f} {s[0]:7d} {s[1]:7d} {s[2]:7d} {s[3]:7d} {s[4]:7d}  {p.split('/maps/')[1]}")
    print("ratios vs stored: opt %.3f webp %.3f; colour/meta split of opt: %.2f/%.2f" %
          (tot[1] / tot[0], tot[2] / tot[0], tot[3] / (tot[3] + tot[4]), tot[4] / (tot[3] + tot[4])))


if __name__ == "__main__":
    main()
