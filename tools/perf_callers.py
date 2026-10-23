"""Who calls a hot leaf function: aggregates `perf script` call chains (perf record --call-graph dwarf) on the testbox.

    perf script -i perf.data | rustfilt | python3 tools/perf_callers.py <leaf-regex> [--top N]

For every sample whose leaf symbol matches, the sample is attributed to the nearest caller frame in a workspace
crate (`bm_*::`), printed with its share of all samples and of the leaf's samples.
"""
import argparse
import collections
import re
import sys


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("leaf")
    ap.add_argument("--top", type=int, default=20)
    a = ap.parse_args()
    leaf = re.compile(a.leaf)
    total = hits = 0
    callers: collections.Counter[str] = collections.Counter()
    frames: list[str] = []

    def flush() -> None:
        nonlocal total, hits
        if not frames:
            return
        total += 1
        if leaf.search(frames[0]):
            hits += 1
            ours = next((f for f in frames[1:] if "bm_" in f), frames[1] if len(frames) > 1 else "?")
            callers[re.sub(r"::h[0-9a-f]{16}$", "", ours)[:160]] += 1
        frames.clear()

    for line in sys.stdin:
        s = line.strip()
        if not s:
            flush()
        elif line[0] in " \t":
            parts = s.split(None, 1)
            if len(parts) == 2:
                frames.append(parts[1].rsplit(" (", 1)[0])
    flush()
    print(f"== {a.leaf}: {hits} of {total} samples ({100 * hits / max(total, 1):.1f}%)")
    for name, n in callers.most_common(a.top):
        print(f"{100 * n / max(total, 1):6.2f}% {100 * n / max(hits, 1):5.1f}%  {name}")


if __name__ == "__main__":
    main()
