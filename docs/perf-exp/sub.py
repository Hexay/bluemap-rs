"""usage: sub.py <folded> <regex> [N] [depth] — within stacks matching regex: share of total, top leaf frames, and top frames just below the match."""
import re,sys,collections
f,pat=sys.argv[1],re.compile(sys.argv[2]); n=int(sys.argv[3]) if len(sys.argv)>3 else 15
leaf=collections.Counter(); child=collections.Counter(); tot=sub=0
def short(x):
    x=re.sub(r"::h[0-9a-f]{16}$","",x); return re.sub(r"<([^<>]*?) as [^<>]*?>",r"\1",x)[:110]
for l in open(f,errors="replace"):
    s,_,w=l.rstrip().rpartition(" "); w=int(w); tot+=w
    fr=s.split(";")
    idx=next((i for i in range(len(fr)-1,-1,-1) if pat.search(fr[i])),None)
    if idx is None: continue
    sub+=w; leaf[short(fr[-1])]+=w
    for x in set(short(y) for y in fr[idx+1:]): child[x]+=w
print(f"## {sys.argv[2]}: {sub/tot*100:.1f}% of total")
print(" leaf:"); [print(f"  {v/tot*100:5.2f}% {k}") for k,v in leaf.most_common(n)]
print(" incl below:"); [print(f"  {v/tot*100:5.2f}% {k}") for k,v in child.most_common(n)]
