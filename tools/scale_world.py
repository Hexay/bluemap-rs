"""Scaling of render CPU, RSS and storage with map size: the 4096² real world (tools/real_world.py, docs/14) tiled
k×k times by hardlinking its 8×8 core regions (no extra disk; chunk positions come from the region slot, so copies
are valid, only with seams). Run on the testbox under `flock ~/bench.lock`.

Usage: py -3 tools/scale_world.py tile <k>                          work/scale/world-<k>: (4096·k)² blocks
       py -3 tools/scale_world.py render <k> [--format optimized] [--exe P] [--heaptrack] [-n 1]
                                                                   timed forced render into work/scale/<k>-<format>
       py -3 tools/scale_world.py noop <k> [--format optimized]    timed `-r` over that unchanged output (scan cost)
Timings go to docs/results/bench.jsonl as scale-<k>-<format>[-noop], with output size and file count.
"""
import argparse
import json
import os
import shutil
import tempfile
from pathlib import Path

from accept import EXE, MC, WORK, prepare, set_conf
from bench import record, run_once
from real_world import SERVER, ensure_config, tree_size

SCALE = WORK / "scale"
OVERWORLD = Path("dimensions") / "minecraft" / "overworld" / "region"
CORE = range(-4, 4)  # region coordinates of the fully generated 4096² area


def world_dir(k: int) -> Path:
    return SCALE / f"world-{k}"


def tile(k: int) -> None:
    src, out = SERVER / "world", world_dir(k)
    shutil.rmtree(out, ignore_errors=True)
    (out / OVERWORLD).mkdir(parents=True)
    shutil.copy2(src / "level.dat", out / "level.dat")
    n = len(CORE)
    for rz in range(-4 * k, 4 * k):
        for rx in range(-4 * k, 4 * k):
            sx, sz = rx % n + CORE.start, rz % n + CORE.start
            os.link(src / OVERWORLD / f"r.{sx}.{sz}.mca", out / OVERWORLD / f"r.{rx}.{rz}.mca")
    print(f"tiled   {out}: {(2 * 4 * k) ** 2} regions, {4096 * k}² blocks")


def run_dir(k: int, fmt: str) -> Path:
    return SCALE / f"{k}-{fmt}"


def timed(label: str, cwd: Path, cmd: list[str], n: int, reset) -> None:
    runs = []
    with tempfile.TemporaryDirectory() as tmp:
        for i in range(n):
            reset()
            runs.append(run_once(cmd, Path(tmp) / f"t{i}.json", cwd))
            r = runs[-1]
            print(f"run {i + 1}/{n}: wall {r['wall']:.1f}s  cpu {r['cpu']:.1f}s  peak {r['peak_mb']:.0f} MB", flush=True)
    files, size = tree_size(cwd / "web" / "maps")
    print(f"output  {files} files, {size / 2**20:.1f} MiB")
    record(label, runs, output_mib=round(size / 2**20, 1), output_files=files)


def wrap(exe: Path, heaptrack: Path | None) -> list[str]:
    return [str(exe.resolve())] if heaptrack is None else ["heaptrack", "-o", str(heaptrack), str(exe.resolve())]


def render(k: int, fmt: str, exe: Path, heaptrack: bool, n: int) -> None:
    if not world_dir(k).is_dir():
        tile(k)
    ensure_config()
    cwd = prepare("real", f"../scale/{k}-{fmt}")
    set_conf(cwd / "config" / "maps" / "real.conf", "world", json.dumps(world_dir(k).as_posix()))
    set_conf(cwd / "config" / "storages" / "file.conf", "format", fmt)
    (cwd / "cmd.json").write_text(json.dumps([str(exe.resolve())]))
    cmd = [*wrap(exe, cwd / "heaptrack" if heaptrack else None), "-c", "config", "-v", MC, "-r", "-f"]
    print(f"== scale {k} ({4096 * k}²) {fmt}: {' '.join(cmd)} (cwd {cwd})", flush=True)
    timed(f"scale-{k}-{fmt}{'-heaptrack' if heaptrack else ''}", cwd, cmd, n,
          lambda: shutil.rmtree(cwd / "web" / "maps", ignore_errors=True))


def noop(k: int, fmt: str) -> None:
    cwd = run_dir(k, fmt)
    cmd = json.loads((cwd / "cmd.json").read_text())
    timed(f"scale-{k}-{fmt}-noop", cwd, [*cmd, "-c", "config", "-v", MC, "-r"], 1, lambda: None)


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("tile").add_argument("k", type=int)
    r = sub.add_parser("render")
    r.add_argument("k", type=int)
    r.add_argument("--format", choices=["compat", "optimized"], default="optimized")
    r.add_argument("--exe", type=Path, default=EXE)
    r.add_argument("--heaptrack", action="store_true")
    r.add_argument("-n", type=int, default=1)
    o = sub.add_parser("noop")
    o.add_argument("k", type=int)
    o.add_argument("--format", choices=["compat", "optimized"], default="optimized")
    a = p.parse_args()
    if a.cmd == "tile":
        tile(a.k)
    elif a.cmd == "render":
        render(a.k, a.format, a.exe, a.heaptrack, a.n)
    else:
        noop(a.k, a.format)


if __name__ == "__main__":
    main()
