"""usage: cpu_areas.py <stacks.folded> [rest-N=12] — share of samples per pipeline area (inferno-collapse-perf --all).
Areas are entry-point frames; a sample goes to the deepest area frame on its stack (so `zstd` inside the BMQ3 encode
counts as zstd, an allocation inside chunk parsing as chunk parse). `rest` lists the leaf frames of unmatched samples."""
import re
import sys
from collections import Counter

AREAS = [
    ("main thread (other)", r"bm_cli::main|bm_engine::update::run$"),
    ("resources load", r"bm_engine::resources|ResourcePack>::load|DataPack>::load|StateCache>::(new|update)|load_gallery|write_textures_file"),
    ("chunk load: read", r"Region>::preload|read_at_most|Region>::open|World>::region$"),
    ("chunk load: parse", r"World>::load_area|Chunk>::parse"),
    ("chunk load: inflate", r"Compression>::decompress_into|ZlibDecoder|inflate"),
    ("tile render", r"HiresRenderer>::render_tile|HiresRenderer>::render_lowres"),
    ("block walk", r"block_pass::render$"),
    ("fully_culled/dark", r"block_pass::fully_culled$|block_pass::dark$"),
    ("Volume::fill", r"Volume>::fill"),
    ("model faces", r"bm_render::resource::render$"),
    ("liquid", r"bm_render::liquid::render$"),
    ("tint/biome", r"Ctx>::tint|View>::uniform_tint|Volume>::biome"),
    ("material sort", r"sort_by_material"),
    ("write_prbm", r"TileModel>::write_prbm"),
    ("stored read-back", r"stored_hires_equals"),
    ("storage write", r"MapStorage>::write_grid|write_grid_encoded"),
    ("bmq3 encode", r"CompactCodec>::encode_into"),
    ("zstd", r"ZSTD_compress|zstd::stream|zstd_safe"),
    ("persist thread", r"bm_engine::persist"),
    ("lowres png", r"encode_png|write_png"),
    ("rayon idle", r"WorkerThread>::wait_until|rayon_core::sleep"),
    ("exit/free", r"drop_in_place<bm_world::world::ChunkArea>|drop_in_place<bm_world::chunk"),
]
PATS = [(n, re.compile(p)) for n, p in AREAS]


def main() -> None:
    nrest = int(sys.argv[2]) if len(sys.argv) > 2 else 12
    area, threads, rest, total = Counter(), Counter(), Counter(), 0
    for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
        stack, _, w = line.rstrip().rpartition(" ")
        w = int(w)
        frames = [re.sub(r"::h[0-9a-f]{16}$", "", f) for f in stack.split(";")]
        total += w
        threads[re.sub(r"[-/?]*\d*(/\d+)?$", "", frames[0])] += w
        hit = None
        for f in frames:
            for n, p in PATS:
                if p.search(f):
                    hit = n
                    break
        area[hit or "rest"] += w
        if hit is None:
            rest[frames[-1][:120]] += w
    for n, _ in AREAS + [("rest", "")]:
        print(f"{n:22} {area[n] / total * 100:5.1f}%")
    print("threads:", ", ".join(f"{k} {v / total * 100:.1f}%" for k, v in threads.most_common(6)))
    print("rest, top leaf frames:")
    for k, v in rest.most_common(nrest):
        print(f"  {v / total * 100:5.1f}% {k}")


if __name__ == "__main__":
    main()
