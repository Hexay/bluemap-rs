"""Writes a BlueMap live/markers.json of rectangular shape markers, like a claims addon's (BlueMap#904: ~9.3k).

usage: python3 tools/gen_markers.py <out markers.json> [--count 9300] [--sets 1] [--radius R] [--seed 904]

Schema and field order are Gson's output for BlueMapAPI ShapeMarker (crates/bm-map/tests/data/markers). Claims are
spread over a square of +-radius blocks (default: grows with count), sizes 8..256 skewed small, five colour styles.
Serving it: docs/18 "Shape-marker batching".
"""

import argparse
import json
import math
import random

STYLES = [
    # (line rgba, fill rgba, line width, depthTest)
    ((255, 85, 85, 1.0), (255, 85, 85, 0.25), 2, False),
    ((85, 170, 255, 1.0), (85, 170, 255, 0.25), 2, False),
    ((85, 255, 85, 1.0), (85, 255, 85, 0.2), 2, False),
    ((255, 215, 0, 1.0), (255, 215, 0, 0.3), 3, False),
    ((170, 0, 170, 0.8), (170, 0, 170, 0.15), 1, True),
]
NAMES = ["Steve", "Alex", "Notch", "Dinnerbone", "Grumm", "jeb_", "Herobrine", "Xisuma", "Mumbo", "Grian"]


def color(rgba):
    r, g, b, a = rgba
    return {"r": r, "g": g, "b": b, "a": a}


def claim(rng, i, radius):
    w = min(256, max(8, int(rng.lognormvariate(math.log(24), 0.7))))
    d = min(256, max(8, int(w * rng.uniform(0.6, 1.6))))
    x = rng.randint(-radius, radius - w)
    z = rng.randint(-radius, radius - d)
    y = rng.choice([63, 64, 64, 70, 80])
    line, fill, width, depth = STYLES[i % len(STYLES)] if rng.random() < 0.9 else STYLES[0]
    owner = f"{rng.choice(NAMES)}{rng.randint(1, 999)}"
    label = f"Claim of {owner}"
    detail = f"<b>{label}</b><br>Size: {w} x {d} ({w * d} blocks)<br>Trusted: {rng.randint(0, 12)}"
    return {
        "shape": [{"x": x, "z": z}, {"x": x + w, "z": z}, {"x": x + w, "z": z + d}, {"x": x, "z": z + d}],
        "holes": [],
        "shapeY": float(y),
        "depthTest": depth,
        "lineWidth": width,
        "lineColor": color(line),
        "fillColor": color(fill),
        "detail": detail,
        "newTab": False,
        "minDistance": 0.0,
        "maxDistance": 10000000.0,
        "type": "shape",
        "label": label,
        "position": {"x": x + w / 2, "y": y, "z": z + d / 2},
        "sorting": 0,
        "listed": True,
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("out")
    ap.add_argument("--count", type=int, default=9300, help="shape markers in all")
    ap.add_argument("--sets", type=int, default=1, help="marker sets they are split over")
    ap.add_argument("--radius", type=int, help="blocks from 0,0 (default: about 60 per sqrt(count))")
    ap.add_argument("--seed", type=int, default=904)
    args = ap.parse_args()
    rng = random.Random(args.seed)
    radius = args.radius or max(512, int(60 * math.sqrt(args.count)))
    sets = {}
    for s in range(args.sets):
        set_id = "claims" if args.sets == 1 else f"claims-{s + 1}"
        sets[set_id] = {
            "label": "Claims" if args.sets == 1 else f"Claims {s + 1}",
            "toggleable": True,
            "defaultHidden": False,
            "sorting": s,
            "markers": {},
        }
    ids = list(sets)
    for i in range(args.count):
        sets[ids[i % len(ids)]]["markers"][f"claim-{i}"] = claim(rng, i, radius)
    body = json.dumps(sets, separators=(",", ":"), ensure_ascii=False)
    with open(args.out, "w", encoding="utf-8") as f:
        f.write(body)
    print(f"{args.out}: {args.count} shape markers in {args.sets} set(s), +-{radius} blocks, {len(body) / 1e6:.1f} MB")


if __name__ == "__main__":
    main()
