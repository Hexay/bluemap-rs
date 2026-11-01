#!/usr/bin/env python3
"""Calculated first-load savings at 50 and 10 Mbit/s from per-tile TSVs: savings.py <tsv>... (first-load view tiles only,
stored PNG as baseline). Candidates need every view tile in the TSV. Rest of the load per the 2026-11-01 load breakdown."""
import sys
from collections import defaultdict

sys.path.insert(0, __file__.rsplit("/", 1)[0])


def in_view(rel):
    s = rel[2:].replace("/", "")  # x-12z34.png
    x, z = s[1:-4].split("z")
    return -4 <= int(x) <= 4 and -3 <= int(z) <= 4


LOAD_TOTAL = 11043.5e3
png = {}
cand = defaultdict(int)
tiles = defaultdict(int)
for path in sys.argv[1:]:
    for line in open(path):
        f = line.rstrip("\n").split("\t")
        if f[0] == "#total" or not in_view(f[2]):
            continue
        png[f[2]] = int(f[3])
        cand[f[0]] += int(f[4])
        tiles[f[0]] += 1
base = sum(png.values())
print(f"view: {len(png)} tiles, {base} PNG bytes")
print("candidate | view bytes | ratio | saved KB | load KB | s saved @50 | s saved @10")
for name, b in cand.items():
    if tiles[name] != len(png):
        continue
    saved = base - b
    print(f"{name} | {b} | {b / base:.3f} | {saved / 1e3:.0f} | {(LOAD_TOTAL - saved) / 1e3:.0f} | {saved * 8 / 50e6:.2f} | {saved * 8 / 10e6:.2f}")
