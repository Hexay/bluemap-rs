"""Benchmark a command: N runs → median/min/max of wall time, process CPU time (user+kernel) and peak
working set. CPU time is the number to trust on a busy machine; wall time is reported for completeness.
If the command writes `--timings {timings}` JSON, per-stage medians are reported too.
Appends a summary line to docs/results/bench.jsonl.

Usage: py -3 tools/bench.py <label> [-n 3] [--clean DIR] -- <command...>
  {timings} in the command is replaced by a temp JSON path.
Example (Java BlueMap baseline; render_serve.py vanilla-512 --no-render --no-serve creates the config first):
  py -3 tools/bench.py java-vanilla-512 -n 3 -- work/downloads/jdk25/bin/java.exe -jar
      work/downloads/bluemap-5.28-cli.jar -c work/bluemap/vanilla-512/config -r -f
"""
import argparse
import ctypes
import json
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
from ctypes import wintypes
from pathlib import Path

from paths import RESULTS, ROOT


class ProcessMemoryCounters(ctypes.Structure):
    _fields_ = [
        ("cb", wintypes.DWORD), ("PageFaultCount", wintypes.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t), ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t),
    ]


def process_stats(handle) -> tuple[float, int]:
    """(user+kernel CPU seconds, peak working set bytes) of an exited process."""
    ft = [wintypes.FILETIME() for _ in range(4)]
    ctypes.windll.kernel32.GetProcessTimes(int(handle), *[ctypes.byref(f) for f in ft])
    to_s = lambda f: ((f.dwHighDateTime << 32) | f.dwLowDateTime) / 1e7
    cpu = to_s(ft[2]) + to_s(ft[3])
    mem = ProcessMemoryCounters()
    mem.cb = ctypes.sizeof(mem)
    ctypes.windll.psapi.GetProcessMemoryInfo(int(handle), ctypes.byref(mem), mem.cb)
    return cpu, mem.PeakWorkingSetSize


def run_once(cmd: list[str], timings_path: Path) -> dict:
    t = time.perf_counter()
    proc = subprocess.Popen([c.replace("{timings}", str(timings_path)) for c in cmd], cwd=ROOT,
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    _, err = proc.communicate()
    wall = time.perf_counter() - t
    if proc.returncode:
        raise SystemExit(f"command failed ({proc.returncode}): {err.decode(errors='replace')[-2000:]}")
    cpu, peak = process_stats(proc._handle)
    stages = json.loads(timings_path.read_text()) if timings_path.exists() else {}
    return {"wall": wall, "cpu": cpu, "peak_mb": peak / 2**20, "stages": stages}


def summary(values: list[float]) -> dict:
    return {"median": round(statistics.median(values), 3), "min": round(min(values), 3), "max": round(max(values), 3)}


def main() -> None:
    # split at "--" by hand: argparse.REMAINDER after a positional also swallows our own options
    argv = sys.argv[1:]
    if "--" not in argv:
        raise SystemExit("usage: bench.py <label> [-n N] [--clean DIR] -- <command...>")
    split = argv.index("--")
    ap = argparse.ArgumentParser()
    ap.add_argument("label")
    ap.add_argument("-n", type=int, default=3)
    ap.add_argument("--clean", help="directory removed before every run")
    args = ap.parse_args(argv[:split])
    cmd = argv[split + 1:]
    # CreateProcess does not resolve a relative executable against cwd
    if (ROOT / cmd[0]).exists():
        cmd[0] = str(ROOT / cmd[0])

    runs = []
    with tempfile.TemporaryDirectory() as tmp:
        for i in range(args.n):
            if args.clean:
                shutil.rmtree(ROOT / args.clean, ignore_errors=True)
            timings = Path(tmp) / f"timings{i}.json"
            runs.append(run_once(cmd, timings))
            r = runs[-1]
            print(f"run {i + 1}/{args.n}: wall {r['wall']:.2f}s  cpu {r['cpu']:.2f}s  peak {r['peak_mb']:.0f} MB")

    stage_names = list(runs[0]["stages"])
    stages = {s: summary([r["stages"][s] for r in runs if s in r["stages"]]) for s in stage_names}
    line = {
        "time": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "commit": subprocess.run(["git", "rev-parse", "--short", "HEAD"], capture_output=True, text=True, cwd=ROOT).stdout.strip(),
        "label": args.label,
        "runs": args.n,
        "wall_s": summary([r["wall"] for r in runs]),
        "cpu_s": summary([r["cpu"] for r in runs]),
        "peak_mb": summary([r["peak_mb"] for r in runs]),
        "stages_s": {s: v["median"] for s, v in stages.items()},
    }
    print(f"{args.label}: wall {line['wall_s']['median']}s  cpu {line['cpu_s']['median']}s  peak {line['peak_mb']['median']:.0f} MB (median of {args.n})")
    for s, v in stages.items():
        print(f"  {s:<26} {v['median']:>8.3f}s  [{v['min']:.3f} .. {v['max']:.3f}]")
    RESULTS.mkdir(exist_ok=True)
    with open(RESULTS / "bench.jsonl", "a") as f:
        f.write(json.dumps(line) + "\n")


if __name__ == "__main__":
    main()
