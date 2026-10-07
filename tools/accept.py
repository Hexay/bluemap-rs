"""Acceptance: render fixtures with our `bluemap` CLI from Java BlueMap's own config folder and compare the webroot
with Java's golden one; then check incremental updates and drop-in over Java's render state.

Usage: py -3 tools/accept.py [fixture ...] [--incremental vanilla ...] [--watch vanilla ...] [--no-build]
Needs: work/bluemap/<fx>/{config,data,web} from tools/render_golden.py (Java BlueMap 5.28, MC 26.3).
Output: work/accept/<fx>/ (fresh each run). Exit 1 if any check fails.
"""
import argparse
import json
import queue
import re
import shutil
import signal
import struct
import subprocess
import sys
import threading
import time
import urllib.request
from pathlib import Path

import hangdump
from bounded import run_bounded

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


def stop(proc: subprocess.Popen) -> None:
    """Terminates a server; kills it if it doesn't exit (a stuck shutdown must not hang the run)."""
    proc.terminate()
    try:
        proc.wait(timeout=30)
    except subprocess.TimeoutExpired:
        print(f"    pid {proc.pid} ignored terminate; killing\n{hangdump.dump(proc.pid, 'stop', EXE)}", flush=True)
        proc.kill()
        proc.wait()


def bluemap(cwd: Path, *flags: str) -> dict[str, tuple[int, int]]:
    """Runs our CLI; returns map id → (tiles rendered, tiles processed incl. skipped/deleted)."""
    start = time.monotonic()
    proc = run_bounded([str(EXE), "-c", "config", "-v", MC, *flags], cwd)
    if proc.returncode:
        sys.exit(f"bluemap failed in {cwd} ({proc.returncode}):\n{proc.stdout[-3000:]}\n{proc.stderr[-3000:]}")
    maps = {m[0]: (int(m[2]), int(m[2]) + int(m[3]) + int(m[4])) for m in MAP_LINE.findall(proc.stdout)}
    print(f"    bluemap {' '.join(flags)}: {maps} ({time.monotonic() - start:.1f}s)", flush=True)
    return maps


def compare(golden: Path, candidate: Path) -> bool:
    proc = run_bounded([str(GOLDEN_EXE), "compare-webroots", str(golden), str(candidate)])
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
    out, conf = incremental_copy(fixture, f"{fixture}-incremental")
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


def incremental_copy(fixture: str, name: str) -> tuple[Path, Path]:
    """A fresh accept dir rendering its own copy of the fixture's world; returns it and the map config."""
    out = prepare(fixture, name)
    conf = next((out / "config" / "maps").glob("*.conf"))
    world = Path(re.search(r'^world:\s*"(.*)"', conf.read_text(), re.M)[1])
    shutil.copytree(world, out / "world")
    set_conf(conf, "world", '"world"')
    return out, conf


def sse_tiles(url: str, into: list[tuple[int, int, int]]) -> None:
    """Appends (x, z, lod) of every `tile` event on the SSE stream at `url` until it closes."""
    event = None
    try:
        with urllib.request.urlopen(url, timeout=600) as stream:
            for raw in stream:
                line = raw.decode().rstrip("\r\n")
                if line.startswith("event: "):
                    event = line[7:]
                elif line.startswith("data: ") and event == "tile":
                    tile = json.loads(line[6:])
                    into.append((tile["x"], tile["y"], tile["lod"]))
    except OSError:
        pass


def watch(fixture: str, failures: list[str], timeout: float = 120, port: int = 8199) -> None:
    """`bluemap -u -w`: a bumped chunk timestamp gets its tiles re-rendered while running and pushed as SSE `tile`
    events; a signal stops it cleanly."""
    print(f"watch {fixture}", flush=True)
    out, conf = incremental_copy(fixture, f"{fixture}-watch")
    set_conf(out / "config" / "core.conf", "update-cooldown", "0")
    set_conf(out / "config" / "webserver.conf", "port", str(port))
    bluemap(out, "-r")
    flags = subprocess.CREATE_NEW_PROCESS_GROUP if sys.platform == "win32" else 0
    proc = subprocess.Popen([str(EXE), "-c", "config", "-v", MC, "-u", "-w"], cwd=out, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, creationflags=flags)
    lines: queue.Queue[str] = queue.Queue()
    threading.Thread(target=lambda: [lines.put(line) for line in proc.stdout], daemon=True).start()

    def wait_for(pattern: re.Pattern | str) -> re.Match | None:
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            try:
                line = lines.get(timeout=1)
            except queue.Empty:
                continue
            print("    | " + line.rstrip(), flush=True)
            if m := re.search(pattern, line):
                return m
        return None

    try:
        check(wait_for("Your maps are now all up-to-date!") is not None, f"{fixture}: -u finishes the initial update", failures)
        events: list[tuple[int, int, int]] = []
        url = f"http://127.0.0.1:{port}/maps/{conf.stem}/live/sse"
        threading.Thread(target=sse_tiles, args=(url, events), daemon=True).start()
        time.sleep(1)
        dim = re.search(r'^dimension:\s*"(.*)"', conf.read_text(), re.M)[1]
        cx, cz = shared_chunk(out / "web", conf.stem)
        for delta in (1, -1):
            start = time.monotonic()
            bump_chunk(region_dir(out / "world", dim), cx, cz, delta)
            m = wait_for(MAP_LINE)
            rendered = int(m[3]) if m else None
            what = f"{fixture}: watched chunk {cx},{cz} {delta:+} -> {rendered} rendered after {time.monotonic() - start:.1f}s (expected {touched_tiles(cx, cz)})"
            check(rendered == touched_tiles(cx, cz), what, failures)
        hires = {(x, z) for x, z, lod in events if lod == 0}
        lods = sorted({lod for _, _, lod in events})
        what = f"{fixture}: SSE pushed {len(events)} tile events, {len(hires)} hires tiles, lods {lods}"
        check(len(hires) == touched_tiles(cx, cz) and len(lods) > 1, what, failures)
        print(f"    distinct tile events: {sorted(set(events))}", flush=True)
        proc.send_signal(signal.CTRL_BREAK_EVENT if sys.platform == "win32" else signal.SIGTERM)
        stopped = wait_for("Stopped.") is not None
        code = proc.wait(timeout=60)
        check(stopped and code == 0, f"{fixture}: stops cleanly on signal (exit {code})", failures)
    finally:
        if proc.poll() is None:
            proc.kill()
    check(compare(WORK / "bluemap" / fixture / "web", out / "web"), f"{fixture}: watched output still equals Java's", failures)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("fixtures", nargs="*", default=DEFAULT_FIXTURES)
    ap.add_argument("--incremental", nargs="*", default=["vanilla", "structures"])
    ap.add_argument("--watch", nargs="*", default=["vanilla"])
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
    for fx in args.watch:
        watch(fx, failures)
    print(f"\n{len(failures)} failed" + "".join(f"\n  {f}" for f in failures))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
