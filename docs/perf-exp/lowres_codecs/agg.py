#!/usr/bin/env python3
"""Summarise lowres_codecs TSVs: agg.py <out.tsv> [--view]. --view: only the first load's tiles (x -4..4, z -3..4)."""
import re
import sys
from collections import defaultdict


def in_view(rel):
    m = re.fullmatch(r"\d/x(-?\d+)z(-?\d+)\.png", rel.replace("\\", "/").replace("/", "", 0)[0] + "/" + rel[2:].replace("/", ""))
    x, z = int(m[1]), int(m[2])
    return -4 <= x <= 4 and -3 <= z <= 4


view = "--view" in sys.argv
rows = defaultdict(lambda: defaultdict(lambda: [0, 0, 0, 0, 0]))
order = []
totals = {}
for line in open(sys.argv[1]):
    f = line.rstrip("\n").split("\t")
    if f[0] == "#total":
        totals[int(f[1])] = (int(f[2]), int(f[3]))
        continue
    name, lod, rel = f[0], int(f[1]), f[2]
    if view and not in_view(rel):
        continue
    if name not in order:
        order.append(name)
    r = rows[name][lod]
    r[0] += 1
    for i, v in enumerate(f[3:7]):
        r[i + 1] += int(v)

print("totals (tiles, PNG bytes):", totals)
print("candidate | tiles L1/L2/L3 | ratio L1 | L2 | L3 | est total MB (ratio x all PNG) | vs PNG | enc ms/tile | dec ms/tile")
for name in order:
    by = rows[name]
    ratios = {l: by[l][2] / by[l][1] for l in by if by[l][1]}
    est = sum(ratios.get(l, 1) * totals[l][1] for l in totals)
    all_png = sum(t[1] for t in totals.values())
    n = sum(by[l][0] for l in by)
    enc = sum(by[l][3] for l in by) / n / 1000
    dec = sum(by[l][4] for l in by) / n / 1000
    print(f"{name} | {'/'.join(str(by[l][0]) for l in (1, 2, 3))} | "
          + " | ".join(f"{ratios.get(l, 0):.3f}" for l in (1, 2, 3))
          + f" | {est / 1e6:.2f} | {est / all_png:.3f} | {enc:.1f} | {dec:.1f}")
