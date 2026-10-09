"""BMQ3 candidates sized against shipped BMQ2. Usage: exp3_variants.py [real|fx] [step] [variant…]"""
import os
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import numpy as np

import exp3_ao as ao
import exp3_lib as lib
import exp3_light as light
import exp3_model as model

baseline = lib.baseline
GEOMETRY = dict.fromkeys(["pos", "pos_esc", "uv", "uv_esc"])


def ao2(t):
    return {"ao": model.ao_packed(t)}


def light4(t):
    return {"light": model.light_packed(t)}


def attrs(t):
    return {**ao2(t), **light4(t)}


def tpl_grid(t):
    """Grid templates replace the position/uv residual records; escapes stay as in BMQ2."""
    ids, table = model.grid_templates(t)
    return {**attrs(t), "pos": None, "uv": None, "tpl": table.tobytes(), "tpl_id": model.ids_bytes(ids, len(table)),
            "cell": model.cell_deltas(t)}


def _quads(t, f):
    """Per-quad streams shared by the float-template variants."""
    return {**attrs(t), **GEOMETRY, "tpl_id": model.ids_bytes(f["ids"], len(f["tpl"])), "cell": model.cell_deltas(t),
            "tpl_exc": f["exc"]}


def tpl_float(t):
    """Exact-float templates, 20 floats each, stored column-wise: no fixed-point records, no escape streams."""
    f = model.float_templates(t)
    return {**_quads(t, f), "tpl": model.columns(model.flat_table(f))}


def tpl_axis(t, hashed=False):
    """Templates as 4 shape ids (x, y, z, uv); each axis has its own table of 4-float shapes."""
    f = model.float_templates(t, hashed)
    shapes = {f"shape_{k}": model.columns(v) for k, v in f["shapes"].items()}
    return {**_quads(t, f), **shapes, "tpl": model.columns(f["tpl"], "<u2")}


def tpl_hash(t):
    """tpl_axis with x/z shapes that may be relative to the block's hash-derived random offset."""
    return tpl_axis(t, hashed=True)


def _steps(t):
    step, dy = model.column_steps(t)
    (col, col_wide), (y, y_wide) = model.escaped(step, "u1"), model.escaped(dy, "i1")
    return col, y, col_wide + y_wide


def cols(t):
    """tpl_hash with cells as (column step, dy) byte streams instead of 4-byte (dx, dz, dy) records."""
    col, y, wide = _steps(t)
    return {**tpl_hash(t), "cell": None, "cell_col": col, "cell_y": y, "cell_wide": wide}


def cols_rec(t):
    """cols with the two bytes interleaved per quad."""
    col, y, wide = _steps(t)
    rec = np.stack([np.frombuffer(col, "u1"), np.frombuffer(y, "u1")], 1).tobytes()
    return {**tpl_hash(t), "cell": rec, "cell_wide": wide}


def ao_geo(t):
    """cols_rec with ao as the residual of the geometric prediction."""
    res = ao.residual(t)
    return {**cols_rec(t), **({} if res is None else {"ao": res})}


def light_map(t):
    """ao_geo with light as the residual of the front-cell prediction."""
    res = light.residual(t)
    return {**ao_geo(t), **({} if res is None else {"light": res})}


def rec3(t):
    """light_map with the template id inside the cell record: (id, column step, dy) per quad."""
    f = model.float_templates(t, hashed=True)
    col, y, wide = _steps(t)
    parts = [model.ids_bytes(f["ids"], len(f["tpl"])), col, y]
    rec = np.concatenate([np.frombuffer(p, "u1").reshape(t["Q"], -1) for p in parts], 1).tobytes()
    return {**light_map(t), "tpl_id": None, "cell": rec}


ALL = ["baseline", "ao2", "light4", "attrs", "tpl_grid", "tpl_float", "tpl_axis", "tpl_hash", "cols", "cols_rec",
       "ao_geo", "light_map", "rec3"]

if __name__ == "__main__":
    corpus = sys.argv[1] if len(sys.argv) > 1 else "real"
    step = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    names = sys.argv[3:] or ALL
    if names[0] == "sweep":
        lib.sweep("exp3_variants", names[1], corpus, step)
        sys.exit()
    if names[0] != "baseline":
        names.insert(0, "baseline")
    lib.run("exp3_variants", names, corpus, step, detail=[n for n in names if n != "baseline"][-2:])
