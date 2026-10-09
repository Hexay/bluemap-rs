"""AO predicted from the tile's own geometry, mirroring `ResourceModelRenderer.testAo` (bm-render
`resource/geometry.rs`): a corner's level is 1 - 0.25 * min(3, occluding neighbours among the up to four blocks
around that corner in front of the face). The decoder cannot see blocks, only quads, so "occluding" is approximated
by "the cell holds a full block face of a material the encoder flagged as occluding" (one bit per material group).
Stored: per vertex (actual - predicted) mod 4, which is mostly zero.
"""
import numpy as np

import exp3_model as model

LEVELS = np.array([255, 191, 127, 63])


def _geometry(t):
    _, q, _, _ = model.quantized(t)
    cell = model.cells(t)
    unit = 1 << t["gp"]
    local = q - (cell * unit)[:, None, :]
    side = (local == unit).astype(np.int64) - (local == 0)
    normal = np.sign(np.cross(q[:, 1] - q[:, 0], q[:, 2] - q[:, 0]))
    aligned = (normal != 0).sum(1) == 1
    extent = np.sort(local.max(1) - local.min(1), 1)
    full_face = aligned & (extent == [0, unit, unit]).all(1) & (np.abs(side).sum(2) == 3).all(1)
    return cell, side, np.where(aligned[:, None], normal, 0), full_face


def predict(t, cell, side, normal, occluder_quad):
    """Predicted occluder count [Q,4] per vertex given which quads mark their cell as occluding."""
    lo = cell.min(0) - 1
    grid = np.zeros(cell.max(0) - lo + 2, bool)
    grid[tuple((cell[occluder_quad] - lo).T)] = True
    at = lambda mask: grid[tuple(np.moveaxis(cell[:, None, :] + side * mask - lo, 2, 0))]
    toward = side * normal[:, None, :]
    x, y, z = toward[..., 0], toward[..., 1], toward[..., 2]
    a = (x + y > 0) & at([1, 1, 0])
    b = (x + z > 0) & at([1, 0, 1])
    c = (y + z > 0) & at([0, 1, 1])
    front = x + y + z > 0
    corner = front & (at([1, 1, 1]) | (a.astype(int) + b + c >= 2))
    return np.minimum(a.astype(int) + b + c + corner, 3)


def residual(t):
    """-> bytes: material flags + one byte per quad of 2-bit (actual - predicted) codes; None if the tile's ao
    values are not the four testAo levels."""
    if "ao_res" in t:
        return t["ao_res"]
    actual = (255 - t["ao"].astype(np.int64))
    if (actual % 64).any():
        t["ao_res"] = None
        return None
    actual //= 64
    cell, side, normal, full_face = _geometry(t)
    group = np.repeat(np.arange(len(t["groups"])), t["groups"][:, 1])
    flags = np.ones(len(t["groups"]), bool)
    errors = lambda: int((predict(t, cell, side, normal, full_face & flags[group]) != actual).sum())
    best = errors()
    for g in np.argsort(-t["groups"][:, 1]):
        flags[g] = False
        now = errors()
        if now < best:
            best = now
        else:
            flags[g] = True
    code = (actual - predict(t, cell, side, normal, full_face & flags[group])) & 3
    packed = code[:, 0] | code[:, 1] << 2 | code[:, 2] << 4 | code[:, 3] << 6
    t["ao_res"] = np.packbits(flags).tobytes() + packed.astype("u1").tobytes()
    t["ao_hit"] = (int((code == 0).sum()), code.size)
    return t["ao_res"]
