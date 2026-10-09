"""Acceptance for the optimized storage format: render a fixture into an optimized file storage, re-render (nothing
to do), serve every hires tile through our webserver (compact blob → PRBM on the fly) and compare with Java's tiles, then
convert back with `--convert-storage file --to compat` and compare the webroot with Java's.

Usage: py -3 tools/accept_optimized.py [fixture ...] [--no-build]   (default: vanilla nether)
Output: work/accept/<fx>-optimized/. Exit 1 if any check fails.
"""
import argparse
import gzip
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

from accept import EXE, MC, ROOT, WORK, bluemap, check, compare, prepare, set_conf, stop


def served_tiles_equal(golden: Path, cwd: Path, port: int) -> bool:
    set_conf(cwd / "config" / "webserver.conf", "port", str(port))
    server = subprocess.Popen([str(EXE), "-c", "config", "-v", MC, "-w"], cwd=cwd, stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL)
    try:
        for _ in range(100):
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=1).read()
                break
            except OSError:
                time.sleep(0.1)
        tiles = sorted((golden / "maps").glob("*/tiles/0/**/*.prbm.gz"))
        for tile in tiles:
            url = f"http://127.0.0.1:{port}/" + tile.relative_to(golden).as_posix().removesuffix(".gz")
            req = urllib.request.Request(url, headers={"Accept-Encoding": "gzip"})
            with urllib.request.urlopen(req, timeout=30) as res:
                body = res.read()
                if res.headers.get("Content-Encoding") == "gzip":
                    body = gzip.decompress(body)
            if body != gzip.decompress(tile.read_bytes()):
                print(f"    served tile differs: {url}")
                return False
        print(f"    {len(tiles)} hires tiles served identically", flush=True)
        return bool(tiles)
    finally:
        stop(server)


def optimized(fixture: str, failures: list[str], port: int) -> None:
    print(f"optimized {fixture}", flush=True)
    out = prepare(fixture, f"{fixture}-optimized")
    set_conf(out / "config" / "storages" / "file.conf", "format", "optimized")
    bluemap(out, "-r")
    maps = out / "web" / "maps"
    check((maps / "bluemap-rs-format.txt").is_file() and not any(maps.glob("*/tiles/0")),
          f"{fixture}: optimized layout (marker, no tiles/0)", failures)
    again = bluemap(out, "-r")
    check(all(r == 0 for r, _ in again.values()), f"{fixture}: optimized second run renders nothing", failures)
    golden = WORK / "bluemap" / fixture / "web"
    check(served_tiles_equal(golden, out, port), f"{fixture}: served tiles equal Java's", failures)
    bluemap(out, "--convert-storage", "file", "--to", "compat")
    check(compare(golden, out / "web"), f"{fixture}: optimized -> compat equals Java's webroot", failures)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("fixtures", nargs="*", default=["vanilla", "nether"])
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release", "-p", "bm-cli", "-p", "bm-golden"], cwd=ROOT, check=True)
    failures: list[str] = []
    for i, fx in enumerate(args.fixtures):
        optimized(fx, failures, 18100 + i)
    print(f"\n{len(failures)} failed" + "".join(f"\n  {f}" for f in failures))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
