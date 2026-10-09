"""Tile model shared by the BMQ3 variants: block cells, quad templates (grid and exact-float) and attribute packing.

Float templates rest on how the mesher forms a position (bm-render `block_pass.rs`): `v = fl32(L + B)`, L the
block-local f32 of the model vertex (after rotation and random offset), B the integer block coordinate inside the
tile. A quad is then (template of 12 L values + 8 uv values, cell B), with no per-quad escapes.
"""
import numpy as np

import bmq
from exp3_lib import rows


def quantized(t):
    """Cached BMQ2 fixed-point values: (pos on-grid mask, pos i64 [Q,4,3], uv mask, uv i64 [Q,4,2])."""
    if "quant" not in t:
        t["quant"] = (*bmq.on_grid(t["pos"], t["gp"]), *bmq.on_grid(t["uv"], t["gu"]))
    return t["quant"]


def cells(t):
    """Block cell [Q,3] of each quad: its centre nudged against the normal, so all faces of a block share a cell."""
    if "cell" not in t:
        q = quantized(t)[1]
        normal = np.cross(q[:, 1] - q[:, 0], q[:, 2] - q[:, 0])
        t["cell"] = np.floor_divide(q.sum(1) - np.sign(normal), 4 << t["gp"])
    return t["cell"]


def first_seen_ids(labels):
    """Arbitrary integer labels -> dense ids numbered by first appearance, plus the index of each id's first row."""
    _, first, inverse = np.unique(labels, return_index=True, return_inverse=True)
    order = np.argsort(first)
    rank = np.empty_like(order)
    rank[order] = np.arange(len(order))
    return rank[inverse], first[order]


def grid_templates(t):
    """-> (template id per quad, template rows i16 [n,20]): cell-local fixed-point positions + fixed-point uv."""
    _, qp, _, qu = quantized(t)
    local = qp - (cells(t) << t["gp"])[:, None, :]
    row = np.concatenate([local.reshape(t["Q"], 12), qu.reshape(t["Q"], 8)], 1)
    _, labels = np.unique(rows(row), return_inverse=True)
    ids, first = first_seen_ids(labels)
    return ids, row[first].astype("<i2")


def axis_shapes(pos, B, d=None):
    """One axis of every quad, pos f32 [Q,4], block coordinate B f32 [Q], optional per-quad offset d f32 [Q] ->
    (representative quad per quad or -1, shapes f32 [Q,4] by representative), where
    `pos[q] == fl32(fl32(shape[rep[q]] + d[q]) + B[q])` bit for bit.

    Quads sharing a coarse key share the candidate whose coordinates are smallest (most fraction bits kept); those it
    does not reproduce try the next candidate. Only quads that reproduce themselves are candidates."""
    Q = len(pos)
    d = np.zeros(Q, np.float32) if d is None else d
    local = (pos.astype(np.float64) - B[:, None]).astype(np.float32)
    shape = (local.astype(np.float64) - d[:, None]).astype(np.float32)
    exact = lambda rep, quad: (((shape[rep] + d[quad, None]) + B[quad, None]).view(np.uint32)
                               == pos[quad].view(np.uint32)).all(1)
    groups, group = np.unique(rows(np.round(shape.astype(np.float64) * 4096).astype(np.int64)), return_inverse=True)
    magnitude = np.abs(pos).max(1)
    everyone = np.arange(Q)
    candidate = exact(everyone, everyone)
    rep_of = np.full(Q, -1)
    todo = everyone
    while len(todo):
        pool = todo[candidate[todo]]
        order = pool[np.lexsort((magnitude[pool], group[pool]))]
        g = group[order]
        lead = np.r_[True, g[1:] != g[:-1]][:len(g)]
        best = np.full(len(groups), -1)
        best[g[lead]] = order[lead]
        rep = best[group[todo]]
        todo, rep = todo[rep >= 0], rep[rep >= 0]
        ok = exact(rep, todo)
        rep_of[todo[ok]] = rep[ok]
        todo = todo[~ok]
    return rep_of, shape


def cells_per_label(label, cell_id):
    """For every row, in how many distinct cells its label occurs (0 for label -1)."""
    pair = np.unique(np.stack([label, cell_id], 1)[label >= 0], axis=0)
    count = np.bincount(pair[:, 0], minlength=max(int(label.max(initial=0)) + 1, 1))
    return np.where(label >= 0, count[label], 0)


def hash_offsets(t):
    """BlueMap's random block offset (dx, dz) f32 [Q,2] of each quad's block: `ResourceModelRenderer.hashToFloat`
    of the world block column, which needs the tile's world origin."""
    cell = cells(t)
    x, z = t["origin"][0] + cell[:, 0], t["origin"][1] + cell[:, 2]

    def offset(seed):
        h = x * 73428767 ^ z * 4382893 ^ seed * 457
        unit = ((h * (h + 456149)) & 0xFFFFFF).astype(np.float32) / np.float32(16777216)
        return (unit - np.float32(0.5)) * np.float32(0.75)

    with np.errstate(over="ignore"):
        return np.stack([offset(123984), offset(345542)], 1)


def float_templates(t, hashed=False):
    """Cached exact-float model of the tile's geometry:
    ids [Q] template per quad; tpl [n,4] the template's x, y, z and uv shape ids; shapes {x,y,z: f32 [m,4],
    uv: f32 [m,8]}; exc = verbatim (axis, quad, 4 floats) for values no shape reproduces.

    `hashed`: x and z shapes may be relative to the block's random offset; a shape id's low bit says which. A quad
    takes the offset form when that shape is shared by more blocks than its plain shape is."""
    name = f"float{int(hashed)}"
    if name in t:
        return t[name]
    Q, cell = t["Q"], cells(t).astype(np.float32)
    cell_id = np.unique(rows(cells(t)), return_inverse=True)[1]
    offsets = hash_offsets(t) if hashed else None
    parts, shapes, exc = [], {}, []
    for a, axis in enumerate("xyz"):
        pos = np.ascontiguousarray(t["pos"][:, :, a])
        rep, table = axis_shapes(pos, cell[:, a])
        form, table_o = np.zeros(Q, np.int64), table
        if hashed and a != 1:
            rep_o, table_o = axis_shapes(pos, cell[:, a], offsets[:, a // 2])
            form = (cells_per_label(rep_o, cell_id) > cells_per_label(rep, cell_id)).astype(np.int64)
            rep = np.where(form == 1, rep_o, rep)
        missed = np.flatnonzero(rep < 0)
        exc.append(bytes([a]) + missed.astype("<u4").tobytes() + pos[missed].astype("<f4").tobytes())
        ids, first = first_seen_ids(np.where(rep < 0, 0, rep) * 2 + form)
        parts.append(ids)
        # a representative's row comes from the table of the form its followers chose, not the one it chose itself
        by_rep = np.maximum(rep[first], 0)
        shapes[axis] = np.where(form[first, None] == 1, table_o[by_rep], table[by_rep]).astype("<f4")
        shapes[axis + "_form"] = form[first].astype("u1")
    _, labels = np.unique(rows(t["uv"].view(np.uint32).reshape(Q, 8)), return_inverse=True)
    ids, first = first_seen_ids(labels)
    parts.append(ids)
    shapes["uv"] = t["uv"][first].reshape(-1, 8).astype("<f4")
    combo = np.stack(parts, 1)
    _, labels = np.unique(rows(combo), return_inverse=True)
    ids, first = first_seen_ids(labels)
    t[name] = {"ids": ids, "tpl": combo[first], "shapes": shapes, "exc": b"".join(exc)}
    return t[name]


def flat_table(f):
    """Templates as 20 floats each (12 block-local position values, 8 uv values)."""
    s, tpl = f["shapes"], f["tpl"]  # plain shapes only
    pos = np.stack([s[name][tpl[:, a]] for a, name in enumerate("xyz")], 2)
    return np.concatenate([pos.reshape(len(tpl), 12), s["uv"][tpl[:, 3]]], 1)


def columns(a, dtype=None):
    """[n,k] -> bytes column by column: similar values end up adjacent."""
    return np.ascontiguousarray((a if dtype is None else a.astype(dtype)).T).tobytes()


def ids_bytes(ids, n):
    return ids.astype("u1" if n <= 256 else "<u2").tobytes()


def cell_deltas(t):
    """Per quad (dx, dz, dy) against the previous quad; i8, i8, i16 when x/z steps fit a byte, else 3 × i16."""
    c = cells(t)
    d = np.diff(c, axis=0, prepend=0)[:, [0, 2, 1]]
    if np.abs(d[:, :2]).max(initial=0) > 127:
        return b"\1" + d.astype("<i2").tobytes()
    rec = np.zeros(len(d), [("x", "i1"), ("z", "i1"), ("y", "<i2")])
    rec["x"], rec["z"], rec["y"] = d[:, 0], d[:, 1], d[:, 2]
    return b"\0" + rec.tobytes()


def column_steps(t):
    """Cells as the mesher emits them: inside a material group the block columns come in scan order and each
    column top-down. -> (column step [Q] >= 0 inside a group, dy [Q]): dy is against the previous quad inside a
    column, and against the first (topmost) quad of the previous column when the column changes."""
    if "steps" in t:
        return t["steps"]
    c, Q = cells(t), t["Q"]
    depth = c[:, 2].max() - c[:, 2].min() + 1
    col = (c[:, 0] - c[:, 0].min()) * depth + (c[:, 2] - c[:, 2].min())
    group_start = np.zeros(Q, bool)
    group_start[np.cumsum(t["groups"][:, 1])[:-1]] = True
    group_start[0] = True
    step = col - np.where(group_start, 0, np.r_[0, col[:-1]])
    run_start = group_start | (step != 0)
    run = np.cumsum(run_start) - 1
    top = np.r_[0, c[run_start, 1]]
    dy = c[:, 1] - np.where(run_start, top[run], np.r_[0, c[:-1, 1]])
    t["steps"] = (step, dy)
    return t["steps"]


def escaped(values, dtype):
    """Small integers as one byte each; the rest as the type's maximum followed up in a second i32 stream."""
    info = np.iinfo(dtype)
    wide = (values < info.min) | (values >= info.max)
    return np.where(wide, info.max, values).astype(dtype).tobytes(), values[wide].astype("<i4").tobytes()


def ao_packed(t):
    """2 bits per vertex when the tile has at most 4 ao levels (always, so far), else the 4 raw bytes."""
    levels, code = np.unique(t["ao"], return_inverse=True)
    if len(levels) > 4:
        return b"\1" + t["ao"].tobytes()
    code = code.reshape(-1, 4).astype(np.uint8)
    packed = code[:, 0] | code[:, 1] << 2 | code[:, 2] << 4 | code[:, 3] << 6
    return b"\0" + bytes([len(levels)]) + levels.tobytes() + packed.astype("u1").tobytes()


def light_packed(t):
    """One byte per quad (block << 4 | sun) when both are 0..15, else the two planes."""
    block, sun = t["block"].astype(np.int64), t["sun"].astype(np.int64)
    if min(block.min(), sun.min()) < 0 or max(block.max(), sun.max()) > 15:
        return b"\1" + t["base"]["light"]
    return b"\0" + (block << 4 | sun).astype("u1").tobytes()
