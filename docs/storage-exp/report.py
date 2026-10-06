"""Print markdown tables from out/analyze.json and out/compact_step1.json (input for docs/09)."""
import json
import os
import sys

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "out")
BIG = ("structures", "structures_numeric", "vanilla_512", "nether")


def load(name):
    with open(os.path.join(OUT, name)) as f:
        return json.load(f)


def pct(a, b):
    return f"{100 * a / b:.1f}%" if b else "-"


def analysis():
    a = load("analyze.json")
    rows = sorted(a["per_map"].items(), key=lambda kv: -kv[1]["gz"]) + [("**total**", a["total"])]
    print("| map | tiles | stored gz MB | 4K-cluster MB | raw PRBM MB | raw/gz | verts M | quads M | B/quad gz |")
    print("|---|---|---|---|---|---|---|---|---|")
    for m, s in rows:
        print(f"| {m} | {s['tiles']} | {s['gz'] / 1e6:.2f} | {s['disk4k'] / 1e6:.2f} | {s['raw'] / 1e6:.1f} | "
              f"{s['raw'] / s['gz']:.1f} | {s['verts'] / 1e6:.2f} | {s.get('quads', 0) / 1e6:.2f} | "
              f"{s['gz'] / max(s.get('quads', 1), 1):.1f} |")
    T = a["total"]
    print("\nattr share of raw:", {k[2:]: pct(v, T["raw"]) for k, v in T.items() if k.startswith("b_")})
    tris, q, verts = T["tris"], T["quads"], T["verts"]
    print("roundtrip", T["roundtrip"], "quad_tiles", T["quad_tiles"], "of nonempty;",
          "uniq full", pct(T["uniq_vert_full"], verts), "uniq pos", pct(T["uniq_vert_pos"], verts),
          "uniq pos+uv+ao", pct(T["uniq_vert_pos_uv_ao"], verts))
    for x in ("normal", "color", "blocklight", "sunlight", "ao"):
        print(f"  {x}: tri-const {pct(T['tri_const_' + x], tris)} quad-const {pct(T['quad_const_' + x], q)}")
    print("normal derivable", pct(T["normal_derivable"], tris), "unit faces", pct(T["unit_face"], q),
          "unit+uv01", pct(T["unit_face_uv01"], q))
    for n in ("pos", "uv"):
        print(f"  {n}: " + " ".join(f"g{g} {pct(T[f'{n}_grid{g}'], T[n + '_vals'])}" for g in (16, 32, 256)),
              "negzero", T[n + "_negzero"], "u16-ok tiles", {g: T.get(f"{n}_u16_tiles_g{g}", 0) for g in (16, 32, 256)})
    print("ranges", a["ranges"], "spans", a["spans"], "normals", a["distinct_normals"], "dedup", a["dedup"])
    for m in BIG:
        s = a["per_map"][m]
        print(f"  {m}: pos g16 {pct(s['pos_grid16'], s['pos_vals'])} g256 {pct(s['pos_grid256'], s['pos_vals'])} "
              f"unit {pct(s['unit_face'], s['quads'])} uniqpos {pct(s['uniq_vert_pos'], s['verts'])}")


def compact():
    c = load(f"compact_step{sys.argv[2] if len(sys.argv) > 2 else 1}.json")
    T = c["total"]
    rows = sorted(c["per_map"].items(), key=lambda kv: -kv[1]["gz_stored"]) + [("**total**", T)]
    print("| map | stored gz | PRBM gz9 | PRBM zstd19 | BMQ raw | BMQ gz9 | BMQ zstd19 | stored/BMQ-zstd |")
    print("|---|---|---|---|---|---|---|---|")
    for m, s in rows:
        g = s["gz_stored"]
        print(f"| {m} | {g / 1e6:.2f} | {s['prbm_gzip9'] / g:.2f} | {s['prbm_zstd19'] / g:.2f} | "
              f"{s['bmq_raw'] / g:.2f} | {s['bmq_gzip9'] / g:.2f} | {s['bmq_zstd19'] / g:.3f} | "
              f"{g / s['bmq_zstd19']:.1f}x |")
    print("\nbrotli subset:", T["prbm_brsub_tiles"], "tiles, stored", T["brsub_gz_stored"],
          {k: round(T[k] / T["brsub_gz_stored"], 3) for k in T if "brsub_" in k and k.endswith(("11", "19", "gzip6"))})
    tg = T["test_gz_stored"]
    print("dict test half:", T["test_tiles"], "tiles", {k: round(T[k] / tg, 3) for k in T if
          k.startswith(("test_prbm_zstd19", "test_bmq_zstd19", "prbm_zstd19_dict", "bmq_zstd19_dict"))},
          "dicts", c["prbm_dicts"], c["bmq_dicts"])
    print("ok", T["ok"], "/", T["tiles"], "fallback", T["fallback"], "grids", c["grids"])
    print(f"ms/tile enc {1e3 * T['enc_s'] / T['tiles']:.1f} dec {1e3 * T['dec_s'] / T['tiles']:.1f} "
          f"gzip6 {1e3 * T['gzip_s'] / T['tiles']:.1f}; MB raw/tile {T['prbm_raw'] / T['tiles'] / 1e6:.2f}")


if __name__ == "__main__":
    {"analysis": analysis, "compact": compact}[sys.argv[1]]()
