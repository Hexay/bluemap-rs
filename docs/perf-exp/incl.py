"""usage: incl.py <events.json> — inclusive share of samples per category (first matching frame from leaf)"""
import json, re, sys
from collections import Counter
CATS = [("caffeine", r"caffeine"), ("prbm-write", r"PRBMWriter|GZIPOutput|Deflater"), ("lowres", r"lowres|Lowres|ImageIO|png|PNG"),
        ("region-read/decompress", r"MCARegion|LinearRegion|Inflater|RegionFile|NBTReader|bluenbt|Chunk_1_|ChunkLoader|MCAChunk"),
        ("getChunk-lookup", r"ChunkGrid.getChunk|MCAWorld.getChunk"), ("neighborhood/block access", r"BlockNeighborhood|ExtendedBlock|Block\.get"),
        ("model-render", r"ModelRenderer|BlockRenderPass|TileModel"), ("render-mgr", r"rendermanager"), ("resources-load", r"resources\.pack|ResourcePack\.load|Loader")]
d = json.load(open(sys.argv[1]))["recording"]["events"]
c = Counter(); thr = Counter()
for ev in d:
    st = (ev["values"].get("stackTrace") or {}).get("frames") or []
    names = [f["method"]["type"]["name"] + "." + f["method"]["name"] for f in st]
    thr[re.sub(r"-\d+$", "", (ev["values"].get("sampledThread") or {}).get("javaName") or "?")] += 1
    for cat, p in CATS:
        if any(re.search(p, n) for n in names):
            c[cat] += 1; break
    else:
        c["other:" + (names[0].split(".")[-2] if names else "?")] += 1
n = len(d)
for k, v in c.most_common(18): print(f"{v/n*100:5.1f}% {k}")
print("threads:", thr.most_common(6))
