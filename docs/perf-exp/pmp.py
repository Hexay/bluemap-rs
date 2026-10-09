"""Sampling without admin rights on Windows (samply needs UAC): minidumps of a forced render at intervals, then per
render thread the top frame and the first bluemap frames, ranked. Needs the exe's .pdb, minidump-stackwalk, dump_syms.
Check the machine is idle first: render threads run at low priority and starve next to other load.

Usage: py -3 docs/perf-exp/pmp.py <bluemap.exe> <fixture dir> <dumps> <seconds between> <scratch dir>"""
import subprocess, sys, time, shutil, re, collections
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools"))
import hangdump
exe, cwd, n, gap = Path(sys.argv[1]), sys.argv[2], int(sys.argv[3]), float(sys.argv[4])
out = Path(sys.argv[5]); out.mkdir(parents=True, exist_ok=True)
shutil.rmtree(Path(cwd) / "web", ignore_errors=True)
p = subprocess.Popen([str(exe), "-c", "config", "-v", "26.3", "-r", "-f"], cwd=cwd, stdout=subprocess.DEVNULL)
time.sleep(2.0)
dumps = []
for i in range(n):
    d = out / f"s{i}.dmp"
    err = hangdump.write_minidump(p.pid, d)
    if err: print(err); break
    dumps.append(d); time.sleep(gap)
p.wait()
sym = out / "syms"
subprocess.run(["dump_syms", "-s", str(sym), str(exe.with_suffix(".pdb"))], capture_output=True)
tops = collections.Counter()
for d in dumps:
    txt = subprocess.run(["minidump-stackwalk", "--symbols-path", str(sym), str(d)], capture_output=True, text=True).stdout
    for block in re.split(r"\n(?=Thread \d+)", txt):
        if "bluemap-render" not in block.split("\n")[0]: continue
        frames = re.findall(r"^\s*\d+\s+(\S+!.*?|\S+ \+ 0x\w+)\s*(?:\[.*)?$", block, re.M)
        frames = [re.sub(r"\s*\[.*", "", f)[:90] for f in frames]
        top = frames[0] if frames else "?"
        app = [f for f in frames if "bluemap" in f.split("!")[0]][:3]
        tops[(top, " < ".join(a.split("!")[1][:70] for a in app))] += 1
tot = sum(tops.values())
for (top, app), c in tops.most_common(40): print(f"{c:4d} {c/tot*100:4.1f}%  {top}\n          {app}")
