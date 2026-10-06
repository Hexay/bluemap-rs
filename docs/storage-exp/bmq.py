"""BMQ1: compact lossless quad encoding of a PRBM tile; decode() + prbm.write() reproduces the PRBM bytes exactly.

Layout: b"BMQ1", u8 mode (0 quads, 1 raw-PRBM fallback), u8 pos_grid_log2, u8 uv_grid_log2, u32 quads, then
streams as [u32 len][bytes]. Quad q = triangles (v0,v1,v2),(v0,v2,v3) = PRBM vertices 6q+{0,1,2,5}.
- pos/uv: fixed-point i16 = value * 2^grid, parallelogram-predicted (v1-v0, v2-v1, v3-(v0+v2-v1)), v0 delta vs
  previous quad's v0; one i16 record per quad (AoS beat SoA byte planes by ~40% under zstd: LZ matches whole quads).
  Off-grid values (incl. -0.0) are escapes: gap-coded index + f32-bit residual vs the grid value, as byte planes.
- ao: 4 bytes/quad, AoS. normal: derived from positions (PRBMWriter formula); color/light: one value per quad, SoA.
  Every quad whose 6 vertices deviate from the prediction is stored verbatim in an exception list.
- groups: (material, quad count) as u32 pairs.
"""
import struct

import numpy as np

import prbm

QV = [0, 1, 2, 5]
MAGIC = b"BMQ1"


def derived_normals(p):
    """PRBMWriter normal: (byte)(normalize((v1-v0)x(v2-v0))*128 - 0.5) in double, per triangle."""
    p = p.reshape(-1, 3, 3).astype(np.float64)
    c = np.cross(p[:, 1] - p[:, 0], p[:, 2] - p[:, 0])
    with np.errstate(invalid="ignore", divide="ignore"):
        v = c / np.linalg.norm(c, axis=1, keepdims=True) * 128 - 0.5
    return np.where(np.isnan(v), 0, np.clip(np.trunc(v), -2**31, 2**31 - 1)).astype(np.int64).astype(np.int8)


def pick_grid(v, choices=(4, 5, 8)):
    """Smallest log2 grid that minimises escapes."""
    best = None
    for g in choices:
        n = int((~on_grid(v, g)[0]).sum())
        if best is None or n < best[1]:
            best = (g, n)
    return best[0]


def on_grid(v, g):
    q = np.clip(np.round(v.astype(np.float64) * (1 << g)), -32768, 32767)
    ok = (q / (1 << g)).astype(np.float32).view(np.uint32) == v.view(np.uint32)
    return ok, q.astype(np.int64)


def predict(q):
    """q[Q,4,k] -> residuals (parallelogram within quad, v0 delta across quads)."""
    r = np.empty_like(q)
    r[:, 0] = np.diff(q[:, 0], axis=0, prepend=0)
    r[:, 1] = q[:, 1] - q[:, 0]
    r[:, 2] = q[:, 2] - q[:, 1]
    r[:, 3] = q[:, 3] - (q[:, 0] + q[:, 2] - q[:, 1])
    return r


def unpredict(r):
    q = np.empty_like(r)
    q[:, 0] = np.cumsum(r[:, 0], axis=0)
    q[:, 1] = r[:, 1] + q[:, 0]
    q[:, 2] = r[:, 2] + q[:, 1]
    q[:, 3] = r[:, 3] + q[:, 0] + q[:, 2] - q[:, 1]
    return q


def enc_fixed(v, g):
    """v float32[Q,4,k] -> (residual planes, escape stream)."""
    ok, q = on_grid(v, g)
    r = predict(q)
    wrapped = ((r + 32768) % 65536 - 32768)  # i16 wraparound; decoder wraps identically
    idx = np.flatnonzero(~ok.reshape(-1))
    gaps = np.diff(idx, prepend=-1).astype("<u4")
    ulp = v.reshape(-1)[idx].view("<i4") - grid_f32(q.reshape(-1)[idx], g).view("<i4")  # wraps mod 2^32
    esc = struct.pack("<I", len(idx)) + byte_planes(gaps) + byte_planes(ulp)
    return wrapped.astype("<i2").tobytes(), esc


def grid_f32(q, g):
    return (q / (1 << g)).astype(np.float32)


def byte_planes(a):
    """4-byte ints -> 4 byte planes (lo..hi); escapes are sparse, small gaps/residuals leave hi planes zero."""
    return np.ascontiguousarray(a.astype("<u4").view(np.uint8).reshape(-1, 4).T).tobytes()


def unbyte_planes(b, n, off):
    return np.frombuffer(b, np.uint8, 4 * n, off).reshape(4, n).T.copy().view("<u4").reshape(-1)


def dec_fixed(pl, esc, g, shape):
    Q, _, k = shape
    r = np.frombuffer(pl, "<i2").reshape(Q, 4, k).astype(np.int64)
    q = ((unpredict(r) + 32768) % 65536 - 32768).reshape(-1)
    v = grid_f32(q, g)
    n = struct.unpack_from("<I", esc)[0]
    idx = np.cumsum(unbyte_planes(esc, n, 4).astype(np.int64)) - 1
    v.view("<u4")[idx] = v.view("<u4")[idx] + unbyte_planes(esc, n, 4 + 4 * n)
    return v.reshape(shape)


def enc_exc(actual, pred):
    """Quads whose 6-vertex rows differ from prediction, stored verbatim."""
    bad = np.flatnonzero((actual != pred).any(axis=(1, 2))).astype("<u4")
    return struct.pack("<I", len(bad)) + bad.tobytes() + np.ascontiguousarray(actual[bad]).tobytes()


def dec_exc(b, pred):
    n = struct.unpack_from("<I", b)[0]
    idx = np.frombuffer(b, "<u4", n, 4)
    pred = pred.copy()
    pred[idx] = np.frombuffer(b, pred.dtype, n * pred[0].size, 4 + 4 * n).reshape(n, *pred.shape[1:])
    return pred


def is_quad_tile(t):
    n = t["n"]
    if n % 6 or (t["groups"][:, 1:] % 6).any():
        return False
    v = np.concatenate([t["position"].view(np.uint32), t["uv"].view(np.uint32), t["ao"].astype(np.uint32)], 1)
    v = v.reshape(-1, 6, v.shape[1])
    return bool((v[:, 3] == v[:, 0]).all() and (v[:, 4] == v[:, 2]).all())


def streams(*bs):
    return b"".join(struct.pack("<I", len(b)) + b for b in bs)


def unstreams(b, off):
    out = []
    while off < len(b):
        n = struct.unpack_from("<I", b, off)[0]
        out.append(b[off + 4:off + 4 + n])
        off += 4 + n
    return out


def face_pred(a, Q):
    """Per-quad value = vertex 0 of the quad; prediction repeats it over 6 vertices."""
    a6 = a.reshape(Q, 6, -1)
    return a6[:, 0], np.repeat(a6[:, :1], 6, axis=1)


def encode(raw):
    t = prbm.parse(raw)
    if t["n"] == 0 or not is_quad_tile(t):
        return MAGIC + bytes([1, 0, 0]) + struct.pack("<I", 0) + raw
    Q = t["n"] // 6
    pos = t["position"].reshape(Q, 6, 3)[:, QV]
    uv = t["uv"].reshape(Q, 6, 2)[:, QV]
    gp, gu = pick_grid(pos), pick_grid(uv)
    pos_pl, pos_esc = enc_fixed(pos, gp)
    uv_pl, uv_esc = enc_fixed(uv, gu)
    ao = t["ao"].reshape(Q, 6)[:, QV].tobytes()
    nrm_pred = np.repeat(derived_normals(t["position"]), 3, axis=0).reshape(Q, 6, 3)
    nrm_exc = enc_exc(t["normal"].reshape(Q, 6, 3), nrm_pred)
    col, col_pred = face_pred(t["color"], Q)
    light = np.concatenate([t["blocklight"], t["sunlight"]], 1)
    lq, l_pred = face_pred(light, Q)
    grp = t["groups"]
    groups = np.stack([grp[:, 0], grp[:, 2] // 6], 1).astype("<u4").tobytes()  # starts are cumulative
    head = MAGIC + bytes([0, gp, gu]) + struct.pack("<I", Q)
    return head + streams(pos_pl, pos_esc, uv_pl, uv_esc, ao, nrm_exc,
                          np.ascontiguousarray(col.T).tobytes(), enc_exc(t["color"].reshape(Q, 6, 3), col_pred),
                          np.ascontiguousarray(lq.T).view(np.uint8).tobytes(),
                          enc_exc(light.reshape(Q, 6, 2), l_pred), groups)


def expand(q4, Q):
    """[Q,4,k] quad vertices -> [Q*6,k] PRBM vertex order."""
    return q4[:, [0, 1, 2, 0, 2, 3]].reshape(Q * 6, -1)


def decode(b):
    """BMQ1 -> PRBM bytes."""
    assert b[:4] == MAGIC
    mode, gp, gu = b[4], b[5], b[6]
    Q = struct.unpack_from("<I", b, 7)[0]
    if mode == 1:
        return bytes(b[11:])
    (pos_pl, pos_esc, uv_pl, uv_esc, ao, nrm_exc, col, col_exc, lq, l_exc, groups) = unstreams(b, 11)
    t = {"n": Q * 6}
    t["position"] = expand(dec_fixed(pos_pl, pos_esc, gp, (Q, 4, 3)), Q)
    t["uv"] = expand(dec_fixed(uv_pl, uv_esc, gu, (Q, 4, 2)), Q)
    t["ao"] = expand(np.frombuffer(ao, np.uint8).reshape(Q, 4)[..., None], Q)
    nrm_pred = np.repeat(derived_normals(t["position"]), 3, axis=0).reshape(Q, 6, 3)
    t["normal"] = dec_exc(nrm_exc, nrm_pred).reshape(-1, 3)
    c = np.frombuffer(col, np.uint8).reshape(3, Q).T
    t["color"] = dec_exc(col_exc, np.repeat(c[:, None], 6, axis=1)).reshape(-1, 3)
    lv = np.frombuffer(lq, np.int8).reshape(2, Q).T
    light = dec_exc(l_exc, np.repeat(lv[:, None], 6, axis=1)).reshape(-1, 2)
    t["blocklight"], t["sunlight"] = light[:, :1], light[:, 1:]
    g = np.frombuffer(groups, "<u4").reshape(-1, 2).astype(np.int64)
    cnt = g[:, 1] * 6
    t["groups"] = np.stack([g[:, 0], np.cumsum(cnt) - cnt, cnt], 1)
    return prbm.write(t)
