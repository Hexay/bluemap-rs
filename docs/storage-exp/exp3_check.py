"""Decodes the float-template geometry model and compares it with the tile bit for bit: every position must come
back as fl32(fl32(shape + offset) + cell) unless the model listed it as an exception. Also reports how often the
AO prediction is right. Usage: exp3_check.py [real|fx] [step]"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import numpy as np

import exp3_ao as ao
import exp3_lib as lib
import exp3_model as model


def main(corpus, step):
    values = wrong = exceptions = offset_quads = quads = ao_hit = ao_all = 0
    for p in lib.paths(corpus, step):
        t = lib.load(p)
        if t is None:
            continue
        f = model.float_templates(t, hashed=True)
        cell, offsets = model.cells(t).astype(np.float32), model.hash_offsets(t)
        tpl = f["tpl"][f["ids"]]
        quads += t["Q"]
        for a, axis in enumerate("xyz"):
            shape = f["shapes"][axis][tpl[:, a]]
            form = f["shapes"][axis + "_form"][tpl[:, a]]
            d = np.where(form == 1, offsets[:, a // 2], np.float32(0)).astype(np.float32)
            decoded = (shape + d[:, None]) + cell[:, a, None]
            assert decoded.dtype == np.float32
            bad = (decoded.view(np.uint32) != np.ascontiguousarray(t["pos"][:, :, a]).view(np.uint32)).any(1)
            values += 4 * t["Q"]
            wrong += int(bad.sum())
            offset_quads += int(form.sum()) if a == 0 else 0
        uv = f["shapes"]["uv"][tpl[:, 3]].reshape(-1, 4, 2)
        wrong += int((uv.view(np.uint32) != t["uv"].view(np.uint32)).any((1, 2)).sum())
        exceptions += (len(f["exc"]) - 3) // 20
        if ao.residual(t) is not None:
            ao_hit += t["ao_hit"][0]
            ao_all += t["ao_hit"][1]
    print(f"{corpus} every {step}th tile: {quads} quads, {wrong} quad axes not reproduced, {exceptions} listed as"
          f" exceptions; {100 * offset_quads / quads:.1f}% of quads use a hash offset on x;"
          f" ao predicted for {100 * ao_hit / ao_all:.2f}% of vertices")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "real", int(sys.argv[2]) if len(sys.argv) > 2 else 16)
