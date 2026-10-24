"""usage: under.py <stacks.folded> <regex> [depth=1] [N=25] — what runs under <regex>: samples whose stack holds it,
grouped by the frame `depth` levels below its deepest match (0 = leaf), as a share of all samples."""
import re
import sys
from collections import Counter


def main() -> None:
    pat = re.compile(sys.argv[2])
    depth = int(sys.argv[3]) if len(sys.argv) > 3 else 1
    n = int(sys.argv[4]) if len(sys.argv) > 4 else 25
    c, total, inside = Counter(), 0, 0
    for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
        stack, _, w = line.rstrip().rpartition(" ")
        w = int(w)
        total += w
        frames = stack.split(";")
        idx = next((i for i in range(len(frames) - 1, -1, -1) if pat.search(frames[i])), None)
        if idx is None:
            continue
        inside += w
        below = frames[idx + 1:]
        key = "(self)" if not below else (below[-1] if depth == 0 else below[min(depth, len(below)) - 1])
        c[re.sub(r"::h[0-9a-f]{16}$", "", key)[:200]] += w
    print(f"{inside / total * 100:5.1f}% total under {sys.argv[2]}")
    for k, v in c.most_common(n):
        print(f"{v / total * 100:5.1f}% {k}")


if __name__ == "__main__":
    main()
