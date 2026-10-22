"""usage: chain.py <stacks.folded> <regex> [depth=5] [N=20] — who calls <regex>: samples grouped by the `depth`
frames above its root-most match (rayon/std/closure plumbing dropped), with that sample share."""
import re
import sys
from collections import Counter

NOISE = re.compile(r"^(rayon|<rayon|crossbeam|std::|<std::|core::|<core::|alloc::|<alloc::|__rust|\[libc|\[unknown|<&)|"
                   r"\{closure|catch_unwind|halt_unwinding|call_once")


def main() -> None:
    pat = re.compile(sys.argv[2])
    depth = int(sys.argv[3]) if len(sys.argv) > 3 else 5
    n = int(sys.argv[4]) if len(sys.argv) > 4 else 20
    c, total = Counter(), 0
    for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
        stack, _, w = line.rstrip().rpartition(" ")
        w = int(w)
        total += w
        frames = [re.sub(r"::h[0-9a-f]{16}$", "", f) for f in stack.split(";")]
        idx = next((i for i, f in enumerate(frames) if pat.search(f)), None)
        if idx is None:
            continue
        above = [f for f in frames[1:idx] if not NOISE.search(f)][-depth:]
        c[" > ".join(above + [frames[idx]])[-400:]] += w
    for k, v in c.most_common(n):
        print(f"{v / total * 100:5.1f}% {k}")


if __name__ == "__main__":
    main()
