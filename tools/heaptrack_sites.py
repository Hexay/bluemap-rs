"""Rank allocation call sites (file:line in our crates) from a heaptrack recording, on the testbox.

    python3 tools/heaptrack_sites.py <heaptrack.*.zst> [--cost calls|temporary] [--top N] [--tiles T]

Streams `heaptrack_print`'s full backtrace report through a pipe (never to disk: it is GBs for a long render) and
attributes each backtrace's count to its leaf-most frame under `crates/bm-*`, printed as `crate/file.rs:line function`
with count, share and per-tile rate (--tiles).
"""
import argparse
import collections
import re
import subprocess

COUNT = re.compile(r"^\s*(\d+) (?:calls|temporary) .*from:$")
AT = re.compile(r"^\s+at (?:\S*/)?(crates/bm-[^:]+):(\d+)$")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("file")
    ap.add_argument("--cost", choices=["calls", "temporary"], default="calls")
    ap.add_argument("--top", type=int, default=25)
    ap.add_argument("--tiles", type=int)
    a = ap.parse_args()
    flags = ["-a", "1", "-T", "0"] if a.cost == "calls" else ["-a", "0", "-T", "1"]
    cmd = ["heaptrack_print", "-f", a.file, "-p", "0", *flags, "-n", "1000000", "-s", "1000000"]
    sites: collections.Counter[str] = collections.Counter()
    pending, prev = 0, ""
    with subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, errors="replace") as p:
        for line in p.stdout:
            if m := COUNT.match(line):
                if pending:
                    sites["(no workspace frame)"] += pending
                pending = int(m.group(1))
            elif pending and (m := AT.match(line)):
                sites[f"{m.group(1).removeprefix('crates/')}:{m.group(2)} {prev.strip()[:70]}"] += pending
                pending = 0
            prev = line
    if pending:
        sites["(no workspace frame)"] += pending
    total = sum(sites.values()) or 1
    per = f" ({total / a.tiles:,.0f}/tile)" if a.tiles else ""
    print(f"== {a.cost}: {total:,}{per}")
    for site, n in sites.most_common(a.top):
        tile = f" {n / a.tiles:8,.1f}/tile" if a.tiles else ""
        print(f"{n:>12,} {100 * n / total:5.1f}%{tile}  {site}")


if __name__ == "__main__":
    main()
