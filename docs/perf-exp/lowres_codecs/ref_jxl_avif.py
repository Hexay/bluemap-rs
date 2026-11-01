#!/usr/bin/env python3
"""Reference only: lossless JXL and AVIF sizes on every Nth tile. ref_jxl_avif.py <tiles dir> <step>. Needs a venv with
pillow + pillow-jxl-plugin. Pixels compared with the source; a mismatch is reported, not fatal."""
import io
import os
import sys
import time

from PIL import Image

try:
    import pillow_jxl  # noqa: F401
except ImportError:
    print("no pillow_jxl")

files = sorted(os.path.join(d, f) for d, _, fs in os.walk(sys.argv[1]) for f in fs if f.endswith(".png"))[:: int(sys.argv[2])]
FORMATS = {
    "JXL e7": ("JXL", {"lossless": True, "effort": 7}),
    "AVIF s6 q100": ("AVIF", {"quality": 100, "speed": 6, "subsampling": "4:4:4"}),
}
for name, (fmt, kw) in FORMATS.items():
    png = out = bad = 0
    t = 0.0
    try:
        for p in files:
            im = Image.open(p).convert("RGBA")
            png += os.path.getsize(p)
            buf = io.BytesIO()
            t0 = time.perf_counter()
            im.save(buf, fmt, **kw)
            t += time.perf_counter() - t0
            out += buf.tell()
            buf.seek(0)
            bad += Image.open(buf).convert("RGBA").tobytes() != im.tobytes()
        print(f"{name}: {len(files)} tiles, PNG {png}, {out} = {out / png:.3f}, enc {t / len(files) * 1000:.0f} ms/tile, not identical {bad}")
    except Exception as e:
        print(f"{name}: failed: {e}")
