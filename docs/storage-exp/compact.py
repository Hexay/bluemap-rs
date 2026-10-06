"""Encode every tile as BMQ1, verify byte-exact PRBM round-trip, time transcode, and measure codec sizes for
both the original PRBM (zero-format-change recompression) and BMQ1.
Usage: compact.py [step]   (step>1 samples every step-th tile). Writes out/compact_step<step>.json."""
import collections
import gzip
import json
import multiprocessing as mp
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import bmq  # noqa: E402
import codec_sizes  # noqa: E402
import prbm  # noqa: E402

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "out")


def prbm_blob(item):
    return prbm.load(item[1])[1]


def bmq_blob(item):
    return bmq.encode(prbm_blob(item))


def verify(item):
    gz, raw = prbm.load(item[1])
    t0 = time.perf_counter()
    enc = bmq.encode(raw)
    t1 = time.perf_counter()
    dec = bmq.decode(enc)
    t2 = time.perf_counter()
    gzip.compress(dec, 6, mtime=0)
    t3 = time.perf_counter()
    return {"ok": int(dec == raw), "fallback": int(enc[4] == 1), "enc_s": t1 - t0, "dec_s": t2 - t1,
            "gzip_s": t3 - t2, "grid": (enc[5], enc[6])}


def main():
    step = int(sys.argv[1]) if len(sys.argv) > 1 else 1
    tiles = prbm.corpus()[::step]
    with mp.Pool(codec_sizes.WORKERS) as pool:
        info = pool.map(verify, tiles, chunksize=2)
        print("verified", flush=True)
        prbm_sizes, prbm_dicts = codec_sizes.measure(tiles, prbm_blob, pool)
        print("prbm measured", flush=True)
        bmq_sizes, bmq_dicts = codec_sizes.measure(tiles, bmq_blob, pool)
    per_map = collections.defaultdict(collections.Counter)
    grids = collections.Counter()
    for (mapid, path), inf, ps, bs in zip(tiles, info, prbm_sizes, bmq_sizes):
        c = per_map[mapid]
        grids[str(inf.pop("grid"))] += 1
        c.update(inf, tiles=1, gz_stored=os.path.getsize(path))
        c.update({"prbm_" + k: v for k, v in ps.items()})
        c.update({"bmq_" + k: v for k, v in bs.items()})
        if "brsub_tiles" in ps:
            c.update(brsub_gz_stored=os.path.getsize(path))
        if "zstd19_dict16k" in ps:  # test-half baselines for a fair dict comparison
            c.update(test_tiles=1, test_gz_stored=os.path.getsize(path),
                     **{f"test_{p}_{k}": v for p, d in (("prbm", ps), ("bmq", bs)) for k, v in d.items()
                        if not k.startswith("zstd19_dict")})
    total = collections.Counter()
    for c in per_map.values():
        total.update(c)
    with open(os.path.join(OUT, f"compact_step{step}.json"), "w") as f:
        json.dump({"per_map": per_map, "total": total, "prbm_dicts": prbm_dicts, "bmq_dicts": bmq_dicts,
                   "grids": grids}, f, indent=1)
    T = total
    mb = lambda k: f"{T[k] / 1e6:.2f}"  # noqa: E731
    print(f"tiles {T['tiles']} ok {T['ok']} fallback {T['fallback']} grids(pos,uv) {dict(grids)}")
    print(f"stored {mb('gz_stored')} prbm raw {mb('prbm_raw')} gz9 {mb('prbm_gzip9')} zstd19 {mb('prbm_zstd19')} "
          f"| bmq raw {mb('bmq_raw')} gz9 {mb('bmq_gzip9')} zstd19 {mb('bmq_zstd19')}")
    print(f"brotli subset ({T['prbm_brsub_tiles']} tiles) stored {mb('brsub_gz_stored')} prbm br11 "
          f"{mb('prbm_brsub_brotli11')} zstd19 {mb('prbm_brsub_zstd19')} | bmq br11 {mb('bmq_brsub_brotli11')} "
          f"zstd19 {mb('bmq_brsub_zstd19')}")
    print(f"test half stored {mb('test_gz_stored')} prbm dict16k {mb('prbm_zstd19_dict16k')} "
          f"dict112k {mb('prbm_zstd19_dict112k')} | bmq zstd19 {mb('test_bmq_zstd19')} "
          f"dict16k {mb('bmq_zstd19_dict16k')} dict112k {mb('bmq_zstd19_dict112k')}")
    print(f"per tile ms: enc {1e3 * T['enc_s'] / T['tiles']:.1f} dec {1e3 * T['dec_s'] / T['tiles']:.1f} "
          f"gzip6 {1e3 * T['gzip_s'] / T['tiles']:.1f}; dicts prbm {prbm_dicts} bmq {bmq_dicts}")


if __name__ == "__main__":
    main()
