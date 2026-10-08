"""Clean force-render with a BlueMap CLI jar; prints wall, CPU, avg busy cores, peak RSS, output size and a tile hash.

Usage: py -3 docs/perf-exp/java_bench.py <bluemap-dir> <threads> [--jar X.jar] [--jfr out.jfr] [--java path/to/java]
  <bluemap-dir> holds config/ (core.conf's render-thread-count is rewritten) and data/; web/maps is wiped.
  tiles_md5 covers every file under web/maps/*/tiles (deterministic; rstate/settings carry timestamps).
  With --jfr, also prints the estimated bytes allocated by render threads (summed allocation-sample weights).
Analyse a recording with agg.py / incl.py / park.py (jfr print --json --stack-depth 20 --events jdk.X rec.jfr > X.json).
"""
import argparse
import hashlib
import json
import re
import shutil
import subprocess
import time
from pathlib import Path

import psutil

DL = Path(__file__).resolve().parents[2] / "work" / "downloads"


def tiles_md5(maps: Path) -> str:
    h = hashlib.md5()
    for f in sorted(p for p in maps.glob("*/tiles/**/*") if p.is_file()):
        h.update(f.relative_to(maps).as_posix().encode())
        h.update(f.read_bytes())
    return h.hexdigest()


def render_thread_alloc_bytes(java: Path, rec: Path) -> int:
    jfr = java.with_name("jfr" + java.suffix)
    # sampled-allocation weights, not ThreadAllocationStatistics: that event only fires at recording start/end,
    # after render threads have exited
    out = subprocess.run([str(jfr), "print", "--json", "--events", "jdk.ObjectAllocationSample", str(rec)],
                         capture_output=True, text=True, check=True).stdout
    return sum(ev["values"]["weight"] for ev in json.loads(out)["recording"]["events"]
               if ((ev["values"].get("eventThread") or {}).get("javaName") or "").startswith("BlueMap-RenderThread"))


ap = argparse.ArgumentParser()
ap.add_argument("dir", type=Path)
ap.add_argument("threads", type=int)
ap.add_argument("--jar", type=Path, default=DL / "bluemap-5.27-cli.jar")
ap.add_argument("--jfr", type=Path)
ap.add_argument("--java", type=Path, default=DL / "jdk25/bin/java.exe")
args = ap.parse_args()

core = args.dir / "config/core.conf"
core.write_text(re.sub(r"render-thread-count: -?\d+", f"render-thread-count: {args.threads}", core.read_text()))
maps = args.dir / "web/maps"
shutil.rmtree(maps, ignore_errors=True)
jfr = [f"-XX:StartFlightRecording=filename={args.jfr.resolve()},settings=profile"] if args.jfr else []

t = time.perf_counter()
p = psutil.Popen([str(args.java), *jfr, "-jar", str(args.jar.resolve()), "-c", "config", "-v", "26.3", "-r", "-f"],
                 cwd=args.dir, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
peak = cpu = 0
while p.poll() is None:
    try:
        peak = max(peak, p.memory_info().rss)
        c = p.cpu_times()
        cpu = c.user + c.system
    except psutil.Error:
        pass
    time.sleep(0.5)
wall = time.perf_counter() - t
size = sum(f.stat().st_size for f in maps.rglob("*") if f.is_file())
line = (f"jar={args.jar.name} threads={args.threads} exit={p.returncode} wall={wall:.0f}s cpu={cpu:.0f}s "
        f"avg_cores={cpu / wall:.1f} peak_rss={peak / 2**20:.0f}MB out={size / 2**20:.1f}MB tiles_md5={tiles_md5(maps)}")
if args.jfr:
    line += f" render_alloc={render_thread_alloc_bytes(args.java, args.jfr) / 2**30:.1f}GiB"
print(line)
