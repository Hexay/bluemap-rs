"""textures.json: size breakdown, duplicate embedded PNGs, PNG re-encode headroom, codec sizes; cross-map identity."""
import base64
import collections
import glob
import gzip
import hashlib
import io
import json
import os
import sys

import brotli
import zstandard
from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import prbm  # noqa: E402


def main():
    seen_files = collections.Counter()
    for p in sorted(x.replace("\\", "/") for x in glob.glob(f"{prbm.ROOT}/*/web/maps/*/textures.json*")):
        stored = open(p, "rb").read()
        raw = gzip.decompress(stored) if p.endswith(".gz") else stored
        seen_files[hashlib.sha256(raw).hexdigest()] += 1
        tex = json.loads(raw)
        pngs = [base64.b64decode(t["texture"].split(",", 1)[1]) for t in tex if t.get("texture")]
        b64 = sum(len(t.get("texture", "")) for t in tex)
        uniq = {hashlib.sha256(x).hexdigest(): x for x in pngs}
        opt = 0
        for x in uniq.values():
            im = Image.open(io.BytesIO(x))
            im.load()
            b = io.BytesIO()
            im.save(b, format="PNG", optimize=True)
            opt += min(b.tell(), len(x))
        anim = sum(1 for t in tex if t.get("animation"))
        print(f"{p.split('/maps/')[1]}: entries={len(tex)} stored={len(stored)} raw={len(raw)} b64={b64} "
              f"png={sum(map(len, pngs))} uniq_png={len(uniq)}/{len(pngs)} uniq_png_bytes={sum(map(len, uniq.values()))} "
              f"uniq_opt={opt} anim={anim} zstd19={len(zstandard.ZstdCompressor(level=19).compress(raw))} "
              f"br11={len(brotli.compress(raw, quality=11))}")
    print("distinct textures.json contents:", len(seen_files), "of", sum(seen_files.values()))


if __name__ == "__main__":
    main()
