"""Baseline sizes + redundancy analysis + tile dedup over the hires corpus. Writes out/analyze.json."""
import collections
import hashlib
import json
import multiprocessing as mp
import os
import sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import prbm  # noqa: E402
from bmq import derived_normals  # noqa: E402
from codec_sizes import WORKERS  # noqa: E402

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "out")
GRIDS = (16, 32, 256)


def n_unique_rows(*cols):
    a = np.ascontiguousarray(np.concatenate([c.reshape(len(c), -1).view(np.uint8) for c in cols], 1))
    return len(np.unique(a.view(f"V{a.shape[1]}")))


def exact_on_grid(v, g):
    q = np.round(v.astype(np.float64) * g)
    back = (q / g).astype(np.float32)
    return (back.view(np.uint32) == v.view(np.uint32)), q


def per_face_const(a, k):
    """Fraction of faces (k vertices each) whose rows are all identical."""
    a = a.reshape(-1, k, a.shape[1])
    return int((a == a[:, :1]).all(axis=(1, 2)).sum())


def unit_faces(p, uv):
    """Quads that are an axis-aligned 1x1 face on the integer block grid with corner UVs in {0,1}."""
    q = p.reshape(-1, 6, 3)[:, [0, 1, 2, 5]].astype(np.float64)
    u = uv.reshape(-1, 6, 2)[:, [0, 1, 2, 5]]
    integral = (q == np.round(q)).all(axis=(1, 2))
    span = q.max(axis=1) - q.min(axis=1)
    shape = np.sort(span, axis=1)
    unit = (shape == [0, 1, 1]).all(axis=1)
    uv01 = np.isin(u, (0.0, 1.0)).all(axis=(1, 2))
    return int((integral & unit).sum()), int((integral & unit & uv01).sum())


def tile_stats(item):
    mapid, path = item
    gz, raw = prbm.load(path)
    t = prbm.parse(raw)
    n, s = t["n"], collections.Counter()
    s.update(tiles=1, gz=len(gz), raw=len(raw), disk4k=prbm.cluster(len(gz)), verts=n, tris=n // 3,
             groups=len(t["groups"]), roundtrip=int(prbm.write(t) == raw))
    for k, v in prbm.attr_bytes(n).items():
        s["b_" + k] = v
    s["b_header_groups"] = len(raw) - sum(prbm.attr_bytes(n).values())
    res = {"stats": s, "hash": hashlib.sha256(raw).hexdigest(), "normals": [], "nonzero": n > 0}
    if n == 0:
        return mapid, res
    p, uv = t["position"], t["uv"]
    key = np.concatenate([p.view(np.uint32), uv.view(np.uint32)], axis=1).reshape(-1, 6, 5)
    quads = (n // 3) % 2 == 0 and (key[:, 3] == key[:, 0]).all() and (key[:, 4] == key[:, 2]).all()
    s["quad_tiles"] = int(quads)
    s["uniq_vert_full"] = n_unique_rows(p, t["normal"], t["color"], uv, t["ao"], t["blocklight"], t["sunlight"])
    s["uniq_vert_pos"] = n_unique_rows(p)
    s["uniq_vert_pos_uv_ao"] = n_unique_rows(p, uv, t["ao"])
    for a in ("normal", "color", "blocklight", "sunlight", "ao"):
        s["tri_const_" + a] = per_face_const(t[a], 3)
        if quads:
            s["quad_const_" + a] = per_face_const(t[a], 6)
    if quads:
        s["quads"] = n // 6
        s["unit_face"], s["unit_face_uv01"] = unit_faces(p, uv)
    s["normal_derivable"] = int((derived_normals(p) == t["normal"][::3]).all(axis=1).sum())
    for name, v in (("pos", p), ("uv", uv)):
        flat = v.reshape(-1)
        s[name + "_vals"] = flat.size
        s[name + "_negzero"] = int((flat.view(np.uint32) == 0x80000000).sum())
        for g in GRIDS:
            ok, q = exact_on_grid(flat, g)
            s[f"{name}_grid{g}"] = int(ok.sum())
            qa = q[ok].reshape(-1)
            if qa.size:
                span = qa.max() - qa.min()
                s[f"{name}_u16_tiles_g{g}"] = int(ok.all() and span < 65536)
                s[f"{name}_span_max_g{g}"] = int(span)  # summed; max taken in report
        s[name + "_min"] = float(v.min())
        s[name + "_max"] = float(v.max())
    res["normals"] = np.unique(t["normal"], axis=0).tolist()
    return mapid, res


def main():
    os.makedirs(OUT, exist_ok=True)
    tiles = prbm.corpus()
    per_map = collections.defaultdict(collections.Counter)
    ranges = collections.defaultdict(lambda: [1e9, -1e9, 1e9, -1e9])
    spans = collections.defaultdict(int)
    hashes = collections.defaultdict(list)
    normals = set()
    with mp.Pool(WORKERS) as pool:
        for mapid, r in pool.imap_unordered(tile_stats, tiles, chunksize=4):
            s = r["stats"]
            for k in [k for k in s if k.endswith(("_min", "_max")) or "_span_max_" in k]:
                v = s.pop(k)
                if "_span_max_" in k:
                    spans[k] = max(spans[k], v)
                else:
                    rg = ranges[k.split("_")[0]]
                    i = 0 if k.endswith("_min") else 1
                    rg[i] = min(rg[i], v) if i == 0 else max(rg[i], v)
            per_map[mapid].update(s)
            hashes[r["hash"]].append(mapid)
            normals.update(map(tuple, r["normals"]))
    total = collections.Counter()
    for c in per_map.values():
        total.update(c)
    dup = [v for v in hashes.values() if len(v) > 1]
    result = {"per_map": per_map, "total": total, "ranges": {k: v[:2] for k, v in ranges.items()},
              "spans": spans, "distinct_normals": len(normals), "normals": sorted(normals),
              "dedup": {"unique_hashes": len(hashes), "dup_groups": len(dup),
                        "dup_tiles_redundant": sum(len(v) - 1 for v in dup),
                        "dup_cross_map_groups": sum(len(set(v)) > 1 for v in dup)}}
    with open(os.path.join(OUT, "analyze.json"), "w") as f:
        json.dump(result, f, indent=1)
    T = total
    print(f"tiles {T['tiles']} roundtrip {T['roundtrip']} quad_tiles {T['quad_tiles']}/{sum(1 for _ in hashes) and T['tiles']}")
    print(f"gz {T['gz']/1e6:.1f}MB raw {T['raw']/1e6:.1f}MB disk4k {T['disk4k']/1e6:.1f}MB verts {T['verts']} quads {T['quads']}")
    print("distinct normals", len(normals), "dedup", result["dedup"], "spans", dict(spans), "ranges", result["ranges"])


if __name__ == "__main__":
    main()
