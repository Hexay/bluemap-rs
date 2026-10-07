"""Acceptance: render fixtures with our `bluemap` CLI from Java BlueMap's own config folder and compare the webroot
with Java's golden one; then check incremental updates and drop-in over Java's render state.

Usage: py -3 tools/accept.py [fixture ...] [--incremental vanilla ...] [--optimized vanilla ...] [--no-build]
Needs: work/bluemap/<fx>/{config,data,web} from tools/render_golden.py (Java BlueMap 5.28, MC 26.3).
Output: work/accept/<fx>/ (fresh each run). Exit 1 if any check fails.
"""
import argparse
import gzip
import re
import shutil
import struct
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# work/ is git-ignored, so a worktree finds it in an enclosing checkout
WORK = next(p / "work" for p in [ROOT, *ROOT.parents] if (p / "work" / "bluemap").is_dir())
EXE = ROOT / "target" / "release" / "bluemap.exe"
GOLDEN_EXE = ROOT / "target" / "release" / "bm-golden.exe"
MC = "26.3"
DEFAULT_FIXTURES = ["vanilla", "structures", "nether", "dimensions", "debug"]
MAP_LINE = re.compile(r"Map '([^']+)': (\d+) regions, (\d+) tiles rendered, (\d+) skipped, (\d+) deleted")


def set_conf(path: Path, key: str, value: str) -> None:
    text = path.read_text()
    new, n = re.subn(rf"^{re.escape(key)}:.*$", f"{key}: {value}", text, flags=re.M)
    path.write_text(new if n else text.rstrip() + f"\n{key}: {value}\n")


def prepare(fixture: str, name: str, web_from: Path | None = None) -> Path:
    """work/accept/<name> with a copy of the fixture's config, the client jar and (optionally) a copied webroot."""
    src = WORK / "bluemap" / fixture
    out = WORK / "accept" / name
    shutil.rmtree(out, ignore_errors=True)
    shutil.copytree(src / "config", out / "config")
    cfg = out / "config"
    set_conf(cfg / "core.conf", "data", '"data"')
    set_conf(cfg / "webapp.conf", "webroot", '"web"')
    set_conf(cfg / "webserver.conf", "webroot", '"web"')
    set_conf(cfg / "storages" / "file.conf", "root", '"web/maps"')
    (out / "data").mkdir()
    jar = f"minecraft-client-{MC}.jar"
    shutil.copy2(src / "data" / jar, out / "data" / jar)
    if web_from:
        shutil.copytree(web_from, out / "web")
    return out


def bluemap(cwd: Path, *flags: str) -> dict[str, tuple[int, int]]:
    """Runs our CLI; returns map id → (tiles rendered, tiles processed incl. skipped/deleted)."""
    start = time.monotonic()
    proc = subprocess.run([str(EXE), "-c", "config", "-v", MC, *flags], cwd=cwd, capture_output=True, text=True)
    if proc.returncode:
        sys.exit(f"bluemap failed in {cwd} ({proc.returncode}):\n{proc.stdout[-3000:]}\n{proc.stderr[-3000:]}")
    maps = {m[0]: (int(m[2]), int(m[2]) + int(m[3]) + int(m[4])) for m in MAP_LINE.findall(proc.stdout)}
    print(f"    bluemap {' '.join(flags)}: {maps} ({time.monotonic() - start:.1f}s)", flush=True)
    return maps


def compare(golden: Path, candidate: Path) -> bool:
    proc = subprocess.run([str(GOLDEN_EXE), "compare-webroots", str(golden), str(candidate)], capture_output=True, text=True)
    print("\n".join("    " + line for line in proc.stdout.splitlines()), flush=True)
    if proc.returncode and not proc.stdout:
        print(proc.stderr)
    return proc.returncode == 0


def check(ok: bool, what: str, failures: list[str]) -> None:
    print(f"  {'ok  ' if ok else 'FAIL'} {what}", flush=True)
    if not ok:
        failures.append(what)


def touched_tiles(cx: int, cz: int) -> int:
    """Hires tiles (32 blocks, offset 2) whose block range covers chunk cx, cz."""
    def axis(c: int) -> int:
        # tile t covers blocks 32t+2 .. 32t+33, i.e. chunks 2t .. 2t+2
        return len([t for t in range(c // 2 - 2, c // 2 + 2) if 2 * t <= c <= 2 * t + 2])
    return axis(cx) * axis(cz)


def region_dir(world: Path, dimension: str) -> Path:
    """bm-world's `dimension_folder`: `dimensions/<ns>/<path>` when present, else the legacy layout."""
    ns, path = dimension.split(":")
    modern = world / "dimensions" / ns / path
    if modern.is_dir():
        return modern / "region"
    legacy = {"overworld": world, "the_nether": world / "DIM-1", "the_end": world / "DIM1"}[path]
    return legacy / "region"


def shared_chunk(webroot: Path, map_id: str) -> tuple[int, int]:
    """Chunk 2t+2 of a rendered hires tile t whose +x, +z and diagonal neighbours are rendered too: it lies under
    exactly those 4 tiles."""
    tiles = set()
    for f in (webroot / "maps" / map_id / "tiles" / "0").rglob("*.prbm*"):
        flat = re.sub(r"\.prbm.*$", "", "".join(f.relative_to(webroot / "maps" / map_id / "tiles" / "0").parts))
        m = re.fullmatch(r"x(-?\d+)z(-?\d+)", flat)
        if m:
            tiles.add((int(m[1]), int(m[2])))
    for x, z in sorted(tiles):
        if {(x + 1, z), (x, z + 1), (x + 1, z + 1)} <= tiles:
            return 2 * x + 2, 2 * z + 2
    sys.exit(f"no 2x2 block of rendered tiles in {webroot}")


def bump_chunk(region_dir: Path, cx: int, cz: int, delta: int) -> None:
    """Adds `delta` to the region-header timestamp of chunk cx, cz."""
    mca = region_dir / f"r.{cx >> 5}.{cz >> 5}.mca"
    data = bytearray(mca.read_bytes())
    i = 4096 + 4 * ((cz & 31) * 32 + (cx & 31))
    (ts,) = struct.unpack_from(">I", data, i)
    struct.pack_into(">I", data, i, ts + delta)
    mca.write_bytes(bytes(data))


def incremental(fixture: str, failures: list[str]) -> None:
    print(f"incremental {fixture}", flush=True)
    out = prepare(fixture, f"{fixture}-incremental")
    conf = next((out / "config" / "maps").glob("*.conf"))
    world = Path(re.search(r'^world:\s*"(.*)"', conf.read_text(), re.M)[1])
    shutil.copytree(world, out / "world")
    set_conf(conf, "world", '"world"')
    bluemap(out, "-r")
    again = bluemap(out, "-r")
    check(all(rendered == 0 for rendered, _ in again.values()), f"{fixture}: second run renders nothing", failures)
    dim = re.search(r'^dimension:\s*"(.*)"', conf.read_text(), re.M)[1]
    regions = region_dir(out / "world", dim)
    cx, cz = shared_chunk(out / "web", conf.stem)
    # bump one chunk's timestamp, then restore it: both are changes, and the final state must equal Java's again
    for delta in (1, -1):
        bump_chunk(regions, cx, cz, delta)
        counts = bluemap(out, "-r").values()
        processed, rendered = sum(p for _, p in counts), sum(r for r, _ in counts)
        expected = touched_tiles(cx, cz)
        what = f"{fixture}: chunk {cx},{cz} timestamp {delta:+} -> {rendered} rendered, {processed} processed (expected {expected})"
        check(processed == expected == rendered, what, failures)
    check(compare(WORK / "bluemap" / fixture / "web", out / "web"), f"{fixture}: incremental output still equals Java's", failures)


def served_tiles_equal(golden: Path, cwd: Path, port: int) -> bool:
    """Starts our webserver on the optimized storage; every hires tile Java rendered must come back (gzip
    transcoded on the fly) as Java's PRBM."""
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
        server.terminate()
        server.wait()


def optimized(fixture: str, failures: list[str]) -> None:
    """Renders into an optimized storage, then checks re-render, serving and conversion back against Java."""
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
    check(served_tiles_equal(golden, out, 18100 + len(failures)), f"{fixture}: served tiles equal Java's", failures)
    bluemap(out, "--convert-storage", "file", "--to", "compat")
    check(compare(golden, out / "web"), f"{fixture}: optimized -> compat equals Java's webroot", failures)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("fixtures", nargs="*", default=DEFAULT_FIXTURES)
    ap.add_argument("--incremental", nargs="*", default=["vanilla", "structures"])
    ap.add_argument("--optimized", nargs="*", default=["vanilla", "nether"])
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--only-incremental", action="store_true")
    args = ap.parse_args()
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release", "-p", "bm-cli", "-p", "bm-golden"], cwd=ROOT, check=True)
    failures: list[str] = []
    for fx in [] if args.only_incremental else args.fixtures:
        print(f"full render {fx}", flush=True)
        out = prepare(fx, fx)
        bluemap(out, "-r")
        check(compare(WORK / "bluemap" / fx / "web", out / "web"), f"{fx}: webroot equals Java's", failures)
        dropin = prepare(fx, f"{fx}-dropin", WORK / "bluemap" / fx / "web")
        maps = bluemap(dropin, "-r")
        check(all(r == 0 for r, _ in maps.values()), f"{fx}: -r over Java's webroot renders nothing (drop-in)", failures)
    for fx in args.incremental:
        incremental(fx, failures)
    for fx in [] if args.only_incremental else args.optimized:
        optimized(fx, failures)
    print(f"\n{len(failures)} failed" + "".join(f"\n  {f}" for f in failures))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
