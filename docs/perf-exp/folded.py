"""usage: folded.py <stacks.folded> [self|incl|callers <regex>] [N] — rank functions from inferno-collapse-perf output.
self: leaf share; incl: share of samples with the function anywhere on the stack; callers: chains ending at <regex>."""
import re
import sys
from collections import Counter

SKIP = re.compile(r"^(rayon|crossbeam|std::|core::ops::function|<rayon|__libc|start_thread|clone3?|\[unknown\])")


def short(frame: str) -> str:
    f = re.sub(r"::h[0-9a-f]{16}$", "", frame)
    f = re.sub(r"<([^<>]*?) as [^<>]*?>", r"\1", f)
    return f.replace("_[i]", "")


def main() -> None:
    path, mode = sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else "self"
    pat = re.compile(sys.argv[3]) if mode == "callers" else None
    n = int(sys.argv[4 if pat else 3]) if len(sys.argv) > (4 if pat else 3) else 30
    c, total = Counter(), 0
    for line in open(path, encoding="utf-8", errors="replace"):
        stack, _, w = line.rstrip().rpartition(" ")
        w = int(w)
        frames = [short(f) for f in stack.split(";")]
        total += w
        if mode == "self":
            c[frames[-1]] += w
        elif mode == "incl":
            for f in set(frames):
                c[f] += w
        else:
            idx = next((i for i in range(len(frames) - 1, -1, -1) if pat.search(frames[i])), None)
            if idx is not None:
                chain = [f for f in frames[: idx + 1] if not SKIP.match(f)][-4:]
                c[" > ".join(chain)] += w
    for k, v in c.most_common(n):
        print(f"{v / total * 100:5.1f}% {k[:220]}")


if __name__ == "__main__":
    main()
