"""Forced full render, Java BlueMap CLI vs our `bluemap`, from copies of the same fixture config (same thread count:
`render-thread-count` in it). Each side gets its own dir under work/bench/; timing via tools/bench.py.

Usage: py -3 tools/bench_render.py [fixture ...] [-n 3] [--only java|rs] [--format compat|optimized]
"""
import argparse
import subprocess
import sys
from pathlib import Path

from accept import EXE, MC, WORK, prepare, set_conf

TOOLS = Path(__file__).resolve().parent
JAVA = WORK / "downloads" / "jdk25" / "bin" / "java.exe"
JAR = WORK / "downloads" / "bluemap-5.28-cli.jar"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("fixtures", nargs="*", default=["structures", "vanilla-512"])
    ap.add_argument("-n", type=int, default=3)
    ap.add_argument("--only", choices=["java", "rs"])
    ap.add_argument("--format", choices=["compat", "optimized"], default="compat", help="our storage format")
    args = ap.parse_args()
    for fx in args.fixtures:
        sides = {"java": [str(JAVA), "-jar", str(JAR)], "rs": [str(EXE)]}
        for side, cmd in sides.items():
            if args.only and side != args.only:
                continue
            label = side if side == "java" or args.format == "compat" else f"{side}-{args.format}"
            cwd = prepare(fx, f"../bench/{fx}-{label}")
            if side == "rs":
                set_conf(cwd / "config" / "storages" / "file.conf", "format", args.format)
            full = [*cmd, "-c", "config", "-v", MC, "-r", "-f"]
            print(f"== {fx} {side}: {' '.join(full)} (cwd {cwd})", flush=True)
            code = subprocess.call([sys.executable, str(TOOLS / "bench.py"), f"{label}-{fx}", "-n", str(args.n),
                                    "--cwd", str(cwd), "--", *full])
            if code:
                sys.exit(code)


if __name__ == "__main__":
    main()
