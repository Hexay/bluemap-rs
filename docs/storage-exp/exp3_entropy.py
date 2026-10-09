"""What an adaptive context-modelling entropy coder could reach on the BMQ3 per-quad streams, against zstd.

Cost model: per context an adaptive symbol distribution (Dirichlet/KT estimator, the exact code length of an
adaptive arithmetic coder that starts flat), summed per tile; no table is transmitted.
Usage: exp3_entropy.py [real|fx] [step]"""
import collections
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import numpy as np
from scipy.special import gammaln

import exp3_lib as lib
import exp3_model as model

ALPHA = (0.02, 0.1, 0.4)


def adaptive_bits(sym, *ctx):
    """Bits to code `sym` adaptively inside each context (best of ALPHA for the tile)."""
    sym = np.unique(sym, return_inverse=True)[1]
    K = int(sym.max()) + 1
    c = np.zeros(len(sym), np.int64)
    for part in ctx:
        part = np.unique(part, return_inverse=True)[1]
        c = c * (int(part.max()) + 1) + part
    c = np.unique(c, return_inverse=True)[1]
    pair = np.unique(c * K + sym, return_counts=True)[1]
    total = np.bincount(c)
    best = None
    for a in ALPHA:
        nats = (gammaln(total + a * K) - gammaln(a * K)).sum() - (gammaln(pair + a) - gammaln(a)).sum()
        best = nats if best is None else min(best, nats)
    return best / np.log(2)


def prev(a, fill=0):
    return np.r_[fill, a[:-1]]


def tile_costs(path):
    t = lib.load(path)
    if t is None:
        return None
    f = model.float_templates(t, hashed=True)
    ids = f["ids"]
    step, dy = model.column_steps(t)
    step, dy = np.clip(step, 0, 40), np.clip(dy, -40, 40)
    same = step == 0
    ao = np.unique(t["ao"], return_inverse=True)[1].reshape(-1, 4)
    ao_byte = ao[:, 0] | ao[:, 1] << 2 | ao[:, 2] << 4 | ao[:, 3] << 6
    light = t["block"].astype(np.int64) << 4 | t["sun"].astype(np.int64)
    color = np.unique(lib.rows(t["color"]), return_inverse=True)[1]
    z = lambda a, dtype="u1": lib.z19(a.astype(dtype).tobytes())
    out = {
        "Q": t["Q"],
        "id zstd": 8 * z(ids, "<u2" if len(f["tpl"]) > 256 else "u1"),
        "id order0": adaptive_bits(ids),
        "id | prev id": adaptive_bits(ids, prev(ids)),
        "id | prev id, same block": adaptive_bits(ids, prev(ids), same),
        "step zstd": 8 * z(step),
        "step order0": adaptive_bits(step),
        "step | prev id": adaptive_bits(step, prev(ids)),
        "step | prev id, id": adaptive_bits(step, prev(ids), ids),
        "dy zstd": 8 * z(dy, "i1"),
        "dy | same block": adaptive_bits(dy, same),
        "dy | same block, id": adaptive_bits(dy, same, ids),
        "dy | same block, id, prev id": adaptive_bits(dy, same, ids, prev(ids)),
        "ao zstd": 8 * z(ao_byte),
        "ao order0": adaptive_bits(ao_byte),
        "ao | id": adaptive_bits(ao_byte, ids),
        "ao | id, prev ao": adaptive_bits(ao_byte, ids, prev(ao_byte)),
        "ao per vertex | id, earlier vertices": sum(
            adaptive_bits(ao[:, v], ids, *(ao[:, u] for u in range(v))) for v in range(4)),
        "light zstd": 8 * z(light),
        "light | prev light": adaptive_bits(light, prev(light)),
        "light | prev light, same block": adaptive_bits(light, prev(light), same),
        "light | prev light, id": adaptive_bits(light, prev(light), ids),
        "color zstd": 8 * lib.z19(model.columns(t["color"])),
        "color | prev color": adaptive_bits(color, prev(color)),
        "color | id": adaptive_bits(color, ids),
    }
    return out


if __name__ == "__main__":
    import multiprocessing
    corpus = sys.argv[1] if len(sys.argv) > 1 else "real"
    step = int(sys.argv[2]) if len(sys.argv) > 2 else 8
    total = collections.Counter()
    with multiprocessing.Pool(lib.WORKERS) as pool:
        for r in pool.imap_unordered(tile_costs, lib.paths(corpus, step), chunksize=4):
            if r:
                total.update(r)
    q = total.pop("Q")
    print(f"{corpus} every {step}th tile, {q / 1e6:.2f} M quads; bits per quad")
    for k, v in total.items():
        print(f"   {k:40s} {v / q:6.3f}")
