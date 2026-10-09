"""Structure statistics that decide which BMQ3 variants are worth building. Usage: exp3_stats.py [real|fx] [step]"""
import collections
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import numpy as np

import bmq
import exp3_lib as lib


rows = lib.rows


def main(corpus, step):
    c = collections.Counter()
    ao_values, light_values = collections.Counter(), collections.Counter()
    per_tile = collections.defaultdict(list)
    for p in lib.paths(corpus, step):
        t = lib.load(p)
        if t is None:
            continue
        Q = t["Q"]
        c["quads"] += Q
        c["tiles"] += 1
        ao = t["ao"]
        ao_values.update(dict(zip(*np.unique(ao, return_counts=True))))
        c["ao const"] += int((ao == ao[:, :1]).all(1).sum())
        c["ao all 255"] += int((ao == 255).all(1).sum())
        per_tile["ao values"].append(len(np.unique(ao)))
        per_tile["ao tuples"].append(len(np.unique(rows(ao))))
        light = np.stack([t["block"], t["sun"]], 1)
        light_values.update(dict(zip(*np.unique(light, return_counts=True))))
        per_tile["light pairs"].append(len(np.unique(rows(light))))
        per_tile["colors"].append(len(np.unique(rows(t["color"]))))
        per_tile["groups"].append(len(t["groups"]))

        okp, qp = bmq.on_grid(t["pos"], t["gp"])
        oku, qu = bmq.on_grid(t["uv"], t["gu"])
        c["pos escaped values"] += int((~okp).sum())
        c["quads with pos escape"] += int((~okp).any((1, 2)).sum())
        c["uv escaped values"] += int((~oku).sum())
        shape = (qp[:, 1:] - qp[:, :1]).reshape(Q, -1)
        per_tile["shapes"].append(len(np.unique(rows(shape))))
        c["quads on grid"] += int(okp.all((1, 2)).sum())
        per_tile["uv tuples"].append(len(np.unique(rows(qu))))
        both = np.concatenate([shape, qu.reshape(Q, -1)], 1)
        per_tile["shape+uv"].append(len(np.unique(rows(both))))
        full = np.concatenate([both, ao], 1)
        per_tile["shape+uv+ao"].append(len(np.unique(rows(full))))
        unit = 1 << t["gp"]
        lo = qp.min(1)
        cell = lo // unit
        per_tile["cells"].append(len(np.unique(rows(cell))))
        c["same cell as previous quad"] += int((cell[1:] == cell[:-1]).all(1).sum())
        ext = qp.max(1) - lo
        c["quad within one block"] += int((ext <= unit).all(1).sum())
        c["unit face"] += int((np.sort(ext, 1) == [0, unit, unit]).all(1).sum())
        if c["tiles"] == 1:
            print("first tile: grid", t["gp"], t["gu"], "groups", len(t["groups"]), "cells of first 40 quads (x y z):")
            print(np.concatenate([cell[:40], np.repeat(np.arange(len(t["groups"])), t["groups"][:, 1])[:40, None]], 1).T)

    q = c["quads"]
    print(f"{corpus}: {c['tiles']} tiles, {q} quads")
    for k, v in c.items():
        if k not in ("quads", "tiles"):
            print(f"   {k:28s} {v:10d}  {100 * v / q:6.2f}% of quads")
    for k, v in per_tile.items():
        v = np.array(v)
        print(f"   per tile {k:14s} median {int(np.median(v)):6d}  p90 {int(np.percentile(v, 90)):6d}  max {v.max():6d}")
    print("   ao values:", sorted(((int(k), v) for k, v in ao_values.items()), key=lambda e: -e[1])[:12], len(ao_values))
    print("   light values (block/sun):", sorted(((int(k), v) for k, v in light_values.items()), key=lambda e: -e[1])[:20])


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "real", int(sys.argv[2]) if len(sys.argv) > 2 else 1)
