"""Run a command (Linux) and sample its RSS over time; print a summary instead of a dump.

    python3 tools/rss_sample.py [--interval S] [--csv FILE] [--buckets N] [--skip S] -- <command…>

Prints mean/p50/p95/max RSS (MiB) over the run (after `--skip` seconds), the CPU seconds (user+sys from wait4), wall,
and an N-bucket timeline of mean RSS so creep is visible. `--csv` keeps every sample (t, rss_kib, cpu_s) for plots.
The child's stdout/stderr pass through; SIGINT/SIGTERM are forwarded so `-u` runs can be ended cleanly.
"""
import argparse
import os
import signal
import subprocess
import sys
import threading
import time

TICK = os.sysconf("SC_CLK_TCK")


def status(pid: int) -> tuple[int | None, int]:
    """(VmRSS KiB, thread count) from /proc/<pid>/status."""
    rss, threads = None, 0
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    rss = int(line.split()[1])
                elif line.startswith("Threads:"):
                    threads = int(line.split()[1])
    except OSError:
        pass
    return rss, threads


def cpu_s(pid: int) -> float | None:
    try:
        with open(f"/proc/{pid}/stat") as f:
            fields = f.read().rsplit(")", 1)[1].split()
        return (int(fields[11]) + int(fields[12])) / TICK
    except (OSError, IndexError):
        return None


def pct(sorted_vals: list[int], p: float) -> int:
    return sorted_vals[min(len(sorted_vals) - 1, int(p * len(sorted_vals)))]


def summarize(samples: list[tuple[float, int, float]], skip: float, buckets: int) -> str:
    kept = [s for s in samples if s[0] >= skip] or samples
    if not kept:
        return "rss: no samples"
    vals = sorted(s[1] for s in kept)
    mib = lambda k: k / 1024
    mean = sum(vals) / len(vals)
    out = [
        f"rss MiB: mean {mib(mean):.0f}  p50 {mib(pct(vals, .5)):.0f}  p95 {mib(pct(vals, .95)):.0f}  "
        f"max {mib(vals[-1]):.0f}  (n={len(vals)}, t>={skip:g}s)  threads max {max(s[3] for s in kept)}"
    ]
    t0, t1 = kept[0][0], kept[-1][0]
    width = (t1 - t0) / buckets if t1 > t0 else 1
    line, threads = [], []
    for b in range(buckets):
        part = [s for s in kept if t0 + b * width <= s[0] < t0 + (b + 1) * width or (b == buckets - 1 and s[0] == t1)]
        line.append(f"{mib(sum(s[1] for s in part) / len(part)):.0f}" if part else "-")
        threads.append(str(max(s[3] for s in part)) if part else "-")
    out.append(f"timeline ({width:.0f}s buckets, MiB): " + " ".join(line))
    out.append("threads (bucket max): " + " ".join(threads))
    return "\n".join(out)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--interval", type=float, default=0.5)
    ap.add_argument("--csv")
    ap.add_argument("--buckets", type=int, default=20)
    ap.add_argument("--skip", type=float, default=0.0)
    ap.add_argument("cmd", nargs=argparse.REMAINDER)
    a = ap.parse_args()
    cmd = a.cmd[1:] if a.cmd[:1] == ["--"] else a.cmd
    start = time.monotonic()
    child = subprocess.Popen(cmd)
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, lambda s, _f: child.send_signal(s))
    samples: list[tuple[float, int, float]] = []
    done = threading.Event()

    def sample():
        while not done.is_set():
            (r, threads), c = status(child.pid), cpu_s(child.pid)
            if r is not None:
                samples.append((time.monotonic() - start, r, c or 0.0, threads))
            done.wait(a.interval)

    sampler = threading.Thread(target=sample, daemon=True)
    sampler.start()
    _, wait_status, usage = os.wait4(child.pid, 0)
    child.returncode = os.waitstatus_to_exitcode(wait_status)
    done.set()
    sampler.join()
    wall = time.monotonic() - start
    if a.csv:
        with open(a.csv, "w") as f:
            f.write("t_s,rss_kib,cpu_s,threads\n")
            f.writelines(f"{t:.2f},{r},{c:.2f},{n}\n" for t, r, c, n in samples)
    print(f"== exit {child.returncode}  wall {wall:.1f}s  cpu {usage.ru_utime + usage.ru_stime:.1f}s "
          f"(user {usage.ru_utime:.1f} sys {usage.ru_stime:.1f})  maxrss {usage.ru_maxrss / 1024:.0f} MiB", file=sys.stderr)
    print(summarize(samples, a.skip, a.buckets), file=sys.stderr)
    return child.returncode


if __name__ == "__main__":
    sys.exit(main())
