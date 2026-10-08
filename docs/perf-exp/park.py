"""usage: park.py <ThreadPark.json> — total parked seconds by first BlueMap frame, per thread group"""
import json, re, sys
from collections import Counter
def secs(s):
    m = re.match(r"PT(?:(\d+)H)?(?:(\d+)M)?(?:([\d.]+)S)?", s); h, mi, se = m.groups()
    return int(h or 0) * 3600 + int(mi or 0) * 60 + float(se or 0)
c = Counter()
for ev in json.load(open(sys.argv[1]))["recording"]["events"]:
    v = ev["values"]; t = re.sub(r"-\d+$", "", (v.get("eventThread") or {}).get("javaName") or "?")
    if not t.startswith(sys.argv[2] if len(sys.argv)>2 else "BlueMap-Render"): continue
    fr = [f["method"]["type"]["name"].split("/")[-1] + "." + f["method"]["name"] + ":" + str(f.get("lineNumber")) for f in (v.get("stackTrace") or {}).get("frames", [])]
    bm = [f for f in fr if not re.match(r"(Unsafe|LockSupport|AbstractQueued|ForkJoin|Thread|ReentrantLock|Condition|.*Sync|Object\.)", f)]
    c[" < ".join(bm[:4])] += secs(v["duration"])
tot = sum(c.values())
print(f"render-thread parked total {tot:.0f}s")
for k, s in c.most_common(10): print(f"{s:7.0f}s {k}")
