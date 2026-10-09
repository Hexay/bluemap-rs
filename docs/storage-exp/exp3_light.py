"""Light predicted from earlier quads. A face's light is max(own block, neighbour in face direction) (bm-render
`resource/mod.rs`), so every face bounds the light of the cell it looks into from above. The decoder keeps the lowest
light seen per looked-into cell. A quad predicts max(front cell, own cell); an unseen front cell is predicted from
its known neighbours by Minecraft's propagation rule (one less per step, sunlight 15 falls unchanged), else from the
previous quad. Stored: per quad (actual - predicted) mod 16 for block and sun.
"""
import numpy as np

import exp3_model as model

NEIGHBOURS = [(0, 1, 0), (0, -1, 0), (1, 0, 0), (-1, 0, 0), (0, 0, 1), (0, 0, -1)]


def residual(t):
    """-> one byte per quad (block residual << 4 | sun residual), or None if a light value is outside 0..15."""
    if "light_res" in t:
        return t["light_res"]
    block, sun = t["block"].astype(np.int64), t["sun"].astype(np.int64)
    if min(block.min(), sun.min()) < 0 or max(block.max(), sun.max()) > 15:
        t["light_res"] = None
        return None
    _, q, _, _ = model.quantized(t)
    normal = np.sign(np.cross(q[:, 1] - q[:, 0], q[:, 2] - q[:, 0]))
    own = model.cells(t) - (model.cells(t).min(0) - 2)
    front = own + np.where(((normal != 0).sum(1) == 1)[:, None], normal, 0)
    sy, sz = (own.max(0) + 3).tolist()[1:]
    pack = lambda c: ((c[:, 0] * sy + c[:, 1]) * sz + c[:, 2]).tolist()
    steps = [(dx * sy + dy) * sz + dz for dx, dy, dz in NEIGHBOURS]
    known_block, known_sun = {}, {}
    out = bytearray(t["Q"])
    last = (0, 0)
    for i, (f, o, b, s) in enumerate(zip(pack(front), pack(own), block.tolist(), sun.tolist())):
        pb, ps = known_block.get(f), known_sun.get(f)
        if pb is None:
            near_b = [known_block[f + d] for d in steps if f + d in known_block]
            near_s = [known_sun[f + d] for d in steps if f + d in known_sun]
            pb = max(max(near_b) - 1, 0) if near_b else last[0]
            ps = (15 if known_sun.get(f + steps[0]) == 15 else max(max(near_s) - 1, 0)) if near_s else last[1]
            known_block[f], known_sun[f] = b, s
        else:
            known_block[f], known_sun[f] = min(pb, b), min(ps, s)
        if o != f:
            pb, ps = max(pb, known_block.get(o, 0)), max(ps, known_sun.get(o, 0))
        out[i] = (b - pb & 15) << 4 | (s - ps & 15)
        last = (b, s)
    t["light_res"] = bytes(out)
    return t["light_res"]
