"""usage: agg.py <events.json> <frame-regex|-> [depth] [weightfield] — aggregate stacks: top-N callers chains containing regex"""
import json, re, sys
from collections import Counter
d = json.load(open(sys.argv[1]))["recording"]["events"]
pat = None if sys.argv[2] == "-" else re.compile(sys.argv[2])
depth = int(sys.argv[3]) if len(sys.argv) > 3 else 6
wf = sys.argv[4] if len(sys.argv) > 4 else None
c, tot = Counter(), 0
for ev in d:
    st = (ev["values"].get("stackTrace") or {}).get("frames") or []
    names = [f["method"]["type"]["name"].split(".")[-1] + "." + f["method"]["name"] + ":" + str(f.get("lineNumber")) for f in st]
    w = ev["values"].get(wf, 1) if wf else 1
    if isinstance(w, dict): w = 1
    tot += w
    if pat:
        idx = next((i for i, n in enumerate(names) if pat.search(n)), None)
        if idx is None: continue
        names = names[idx:]
    else:
        names = [n for n in names if not re.match(r"(Unsafe|LockSupport|AbstractQueuedSynchronizer|ForkJoinPool|Thread)\.", n)]
    c[" < ".join(names[:depth])] += w
for k, v in c.most_common(int(sys.argv[5]) if len(sys.argv) > 5 else 15):
    print(f"{v/tot*100:5.1f}% {k}")
