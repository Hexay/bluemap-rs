"""Real-scale parity and benchmarks: fixtures/real (4096² vanilla blocks, 64 regions) rendered by Java BlueMap 5.28 and
by our `bluemap` from copies of one config (render-thread-count = all cores; Java on its default heap). Run on the testbox
under `flock ~/bench.lock`; results in docs/14-real-world-validation.md.

Usage: py -3 tools/real_world.py gen [--batch 32] [--parallel 3]   pregenerate the area batch by batch, then verify it
       py -3 tools/real_world.py verify                            chunk statuses / light data of the world
       py -3 tools/real_world.py edit                              world copy with builds at EDITS, saved by the server
       py -3 tools/real_world.py render <label> [--java | --exe P] [--format optimized] [-n 1]
                                                                   timed forced render into work/real/<label>
       py -3 tools/real_world.py update <label> [-n 1]             timed `-r` of <label>'s output over the edited world,
                                                                   into work/real/<label>-update
       py -3 tools/real_world.py convert <label>                   timed optimized -> compat copy, <label>-compat
       py -3 tools/real_world.py compare <label> <label>           bm-golden compare-webroots (first = golden)
Timings go to docs/results/bench.jsonl as real-<label>[-update], with the storage size of the output.
"""
import argparse
import json
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from collections import Counter, deque
from pathlib import Path

from accept import EXE, GOLDEN_EXE, MC, WORK, prepare, set_conf
from bench import record, run_once
from bench_render import JAR, JAVA
from console import Server
from make_world import forceload_commands, load_fixture, write_server_files

FIXTURE = "real"
SERVER = WORK / "worlds" / FIXTURE
EDITED = WORK / "real" / "world-edited"
REAL = WORK / "real"
HEAP = "8G"
# block x, z of each 48² build: five regions, none at spawn
EDITS = [(-1800, -1700), (-700, 900), (260, -340), (1100, 1500), (1650, -1250)]


def wait_chunks(server: Server, box: tuple[int, int, int, int]) -> None:
    """Until every chunk in `box` is loaded at full status. All tests go out at once: the console runs a tick's
    queued commands together, one round trip per chunk would take a tick each."""
    x0, z0, x1, z1 = box
    todo = [(x, z) for z in range(z0, z1 + 1, 16) for x in range(x0, x1 + 1, 16)]
    while todo:
        for x, z in todo:
            server.send(f"execute if loaded {x} 0 {z}")
        results = [server.wait_for(r"Test (passed|failed)", timeout=600).group(1) for _ in todo]
        todo = [c for c, r in zip(todo, results) if r == "failed"]
        if todo:
            time.sleep(2)


def gen(batch: int, parallel: int) -> None:
    """Force-loads `batch`² chunks at a time, `parallel` batches in flight, releasing each once full: memory stays flat
    and every chunk ends fully generated and lit, except the outermost ring (neighbours of the area).
    The fixture turns off sync-chunk-writes: an fsync per chunk made the final save take hours on spinning disks."""
    spec, _ = load_fixture(FIXTURE)
    if SERVER.exists():
        sys.exit(f"{SERVER} exists; delete it to regenerate")
    write_server_files(SERVER, spec["properties"])
    x0, z0, x1, z1 = spec["area"]
    step = 16 * batch
    boxes = [(x, z, min(x + step, x1 + 1) - 1, min(z + step, z1 + 1) - 1)
             for z in range(z0, z1 + 1, step) for x in range(x0, x1 + 1, step)]
    server = Server(SERVER, HEAP)
    start = time.monotonic()
    try:
        server.wait_for(r"Done \(", timeout=600, echo=True)
        pending: deque = deque()
        for i, box in enumerate(boxes):
            for cmd in forceload_commands(*box):
                server.send(cmd)
            pending.append(box)
            while pending and (len(pending) >= parallel or i == len(boxes) - 1):
                done = pending.popleft()
                wait_chunks(server, done)
                for cmd in forceload_commands(*done):
                    server.send(cmd.replace("forceload add", "forceload remove"))
            print(f"batch {i + 1}/{len(boxes)} queued, {time.monotonic() - start:.0f}s", flush=True)
        server.query("save-all flush", r"Saved the game", timeout=1800)
    finally:
        code = server.stop(timeout=1800)
    print(f"generated {spec['area']} in {time.monotonic() - start:.0f}s (exit {code}, {len(server.errors)} errors)")
    for err in server.errors:
        print(f"ERROR   {err}")
    verify()


def chunk_nbt(region: bytes, i: int) -> bytes | None:
    offset = int.from_bytes(region[4 * i:4 * i + 3], "big") * 4096
    if not offset:
        return None
    length, kind = struct.unpack_from(">IB", region, offset)
    data = region[offset + 5:offset + 4 + length]
    return {1: lambda d: zlib.decompress(d, 31), 2: zlib.decompress, 3: bytes}[kind](data)


def verify() -> None:
    """Counts chunks by Status and those carrying sky light: BlueMap skips or darkens unlit ones (CLAUDE.md)."""
    statuses: Counter = Counter()
    lit = 0
    regions = sorted((SERVER / "world").rglob("region/r.*.mca"))
    for mca in regions:
        data = mca.read_bytes()
        for i in range(1024):
            nbt = chunk_nbt(data, i)
            if nbt is None:
                continue
            m = re.search(rb"\x08\x00\x06Status(..)", nbt)
            n = struct.unpack(">H", m[1])[0] if m else 0
            statuses[nbt[m.end():m.end() + n].decode() if m else "?"] += 1
            lit += b"\x07\x00\x08SkyLight" in nbt
    print(f"{len(regions)} region files, {sum(statuses.values())} chunks, {lit} with SkyLight; status: {dict(statuses)}")


def edit_commands(x: int, z: int) -> list[str]:
    return [
        f"fill {x} 140 {z} {x + 47} 140 {z + 47} minecraft:glass",
        f"fill {x + 8} 40 {z + 8} {x + 23} 110 {z + 23} minecraft:air",
        f"fill {x + 30} 60 {z + 30} {x + 40} 130 {z + 40} minecraft:gold_block hollow",
        f"setblock {x + 2} 141 {z + 2} minecraft:torch",
    ]


def edit() -> None:
    """A copy of the world, opened by the server, EDITS built and saved: what a running server does to a map."""
    shutil.rmtree(EDITED, ignore_errors=True)
    shutil.copytree(SERVER, EDITED)
    server = Server(EDITED, HEAP)
    try:
        server.wait_for(r"Done \(", timeout=600, echo=True)
        for x, z in EDITS:
            for cmd in forceload_commands(x, z, x + 47, z + 47):
                server.send(cmd)
            wait_chunks(server, (x, z, x + 47, z + 47))
            for cmd in edit_commands(x, z):
                server.send(cmd)
        server.query("save-all flush", r"Saved the game", timeout=600)
    finally:
        code = server.stop()
    print(f"edited  {EDITED / 'world'} at {EDITS} (exit {code}, {len(server.errors)} errors)")
    if server.errors or code:
        sys.exit("\n".join(server.errors))


def ensure_config() -> None:
    """Java's config folder for the fixture (render_serve.configure) plus the client jar, so no run downloads."""
    from render_serve import configure
    from setup import download, version_json

    base = configure(FIXTURE)
    client = version_json(MC)["downloads"]["client"]
    download(client["url"], base / "data" / f"minecraft-client-{MC}.jar", client["sha1"])


def tree_size(path: Path) -> tuple[int, int]:
    files = [f for f in path.rglob("*") if f.is_file()]
    return len(files), sum(f.stat().st_size for f in files)


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
    record(f"real-{label}", runs, output_mib=round(size / 2**20, 1), output_files=files)


def render(label: str, exe: Path | None, fmt: str | None, n: int) -> None:
    ensure_config()
    cwd = prepare(FIXTURE, f"../real/{label}")
    if fmt:
        set_conf(cwd / "config" / "storages" / "file.conf", "format", fmt)
    cmd = [str(JAVA), "-jar", str(JAR)] if exe is None else [str(exe.resolve())]
    (cwd / "cmd.json").write_text(json.dumps(cmd))
    print(f"== {label}: {' '.join(cmd)} -r -f (cwd {cwd})", flush=True)
    timed(label, cwd, [*cmd, "-c", "config", "-v", MC, "-r", "-f"], n,
          lambda: shutil.rmtree(cwd / "web" / "maps", ignore_errors=True))


def update(label: str, n: int) -> None:
    src, out = REAL / label, REAL / f"{label}-update"
    cmd = json.loads((src / "cmd.json").read_text())

    def reset() -> None:
        shutil.rmtree(out, ignore_errors=True)
        shutil.copytree(src, out)
        conf = next((out / "config" / "maps").glob("*.conf"))
        set_conf(conf, "world", json.dumps((EDITED / "world").as_posix()))

    timed(f"{label}-update", out, [*cmd, "-c", "config", "-v", MC, "-r"], n, reset)


def convert(label: str) -> None:
    """Timed `--convert-storage file --to compat` of a copy of <label>'s (optimized) output, for comparing."""
    src, out = REAL / label, REAL / f"{label}-compat"
    cmd = json.loads((src / "cmd.json").read_text())

    def reset() -> None:
        shutil.rmtree(out, ignore_errors=True)
        shutil.copytree(src, out)

    timed(f"{label}-to-compat", out, [*cmd, "-c", "config", "-v", MC, "--convert-storage", "file", "--to", "compat"],
          1, reset)


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    g = sub.add_parser("gen")
    g.add_argument("--batch", type=int, default=32, help="chunks per batch side")
    g.add_argument("--parallel", type=int, default=3, help="batches in flight")
    sub.add_parser("verify")
    sub.add_parser("edit")
    r = sub.add_parser("render")
    r.add_argument("label")
    side = r.add_mutually_exclusive_group()
    side.add_argument("--java", action="store_true")
    side.add_argument("--exe", type=Path, default=EXE)
    r.add_argument("--format", choices=["compat", "optimized"])
    r.add_argument("-n", type=int, default=1)
    u = sub.add_parser("update")
    u.add_argument("label")
    u.add_argument("-n", type=int, default=1)
    sub.add_parser("convert").add_argument("label")
    c = sub.add_parser("compare")
    c.add_argument("golden")
    c.add_argument("candidate")
    args = ap.parse_args()
    if args.cmd == "gen":
        gen(args.batch, args.parallel)
    elif args.cmd == "verify":
        verify()
    elif args.cmd == "edit":
        edit()
    elif args.cmd == "render":
        render(args.label, None if args.java else args.exe, args.format, args.n)
    elif args.cmd == "update":
        update(args.label, args.n)
    elif args.cmd == "convert":
        convert(args.label)
    else:
        # no hang limit (accept.compare's): a 64-region webroot takes minutes
        sys.exit(subprocess.call([str(GOLDEN_EXE), "compare-webroots", str(REAL / args.golden / "web"),
                                  str(REAL / args.candidate / "web")]))


if __name__ == "__main__":
    main()
