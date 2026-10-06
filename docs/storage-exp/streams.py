"""Per-stream raw and zstd-19 bytes of BMQ1 over a tile sample. Usage: streams.py [step]"""
import collections, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import zstandard as zstd
import bmq, prbm
NAMES = ["pos", "pos_esc", "uv", "uv_esc", "ao", "normal_exc", "color", "color_exc", "light", "light_exc", "groups"]
step = int(sys.argv[1]) if len(sys.argv) > 1 else 60
raw_t, z_t = collections.Counter(), collections.Counter()
for _, p in prbm.corpus()[::step]:
    b = bmq.encode(prbm.load(p)[1])
    if b[4] == 1:
        continue
    for n, s in zip(NAMES, bmq.unstreams(b, 11)):
        raw_t[n] += len(s); z_t[n] += len(zstd.ZstdCompressor(level=19).compress(s))
tot = sum(z_t.values())
for n in NAMES:
    print(f"{n:11s} raw {raw_t[n]/1e3:9.0f}kB  zstd19 {z_t[n]/1e3:8.0f}kB  {100*z_t[n]/tot:5.1f}%")
