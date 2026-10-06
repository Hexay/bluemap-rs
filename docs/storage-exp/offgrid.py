"""Most common fractional parts of off-grid (not k/256) position values, and which materials carry them."""
import collections, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import numpy as np
import prbm
fr, mats, axes = collections.Counter(), collections.Counter(), collections.Counter()
tot = off = 0
for _, p in prbm.corpus()[::60]:
    t = prbm.parse(prbm.load(p)[1])
    if not t["n"]: continue
    v = t["position"]
    ok = np.round(v.astype(np.float64) * 256) / 256 == v
    tot += v.size; off += (~ok).sum()
    r, c = np.nonzero(~ok)
    axes.update(c.tolist())
    fr.update(np.round(np.mod(v[r, c].astype(np.float64), 1), 5).tolist())
    vm = np.repeat(t["groups"][:, 0], t["groups"][:, 2])
    mats.update(vm[np.unique(r)].tolist())
print("off-grid frac", off / tot, "axes", dict(axes))
print("top fracs", fr.most_common(15))
print("top materials", mats.most_common(10))
