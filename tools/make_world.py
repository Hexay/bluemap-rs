"""Generate a fixture world: fresh server run, force-load the fixture area, apply its commands, save, stop.

Usage: py -3 tools/make_world.py <fixture> [--force] [--mc 1.21.11 [--bluemap 5.28]]
Output: <toolchain worlds>/<fixture>/world (default toolchain: work/worlds/<fixture>/world)
"""
import argparse
import json
import shutil
import sys
import time

from console import Server, ServerTimeout
from paths import DEFAULT, FIXTURES, Toolchain

BASE_PROPERTIES = {
    "level-name": "world",
    "online-mode": "false",
    "server-port": "25599",
    "spawn-protection": "0",
    "max-players": "1",
    "view-distance": "4",
    "simulation-distance": "4",
    # 1.21.2+ pauses an empty server after 60 s, which also stops force-loaded chunks from generating
    "pause-when-empty-seconds": "0",
    # force-loading a patch can take one tick past the 60 s watchdog on slower cores (the testbox)
    "max-tick-time": "-1",
}
FORCELOAD_MAX_CHUNKS = 256
OVERWORLD = "minecraft:overworld"


def load_fixture(name: str) -> tuple[dict, list[str]]:
    d = FIXTURES / name
    spec = json.loads((d / "fixture.json").read_text())
    cmd_file = d / "commands.txt"
    commands = []
    if cmd_file.exists():
        for line in cmd_file.read_text().splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                commands.append(line)
    return spec, commands


def write_server_files(server_dir, properties: dict) -> None:
    server_dir.mkdir(parents=True)
    (server_dir / "eula.txt").write_text("eula=true\n")
    props = {**BASE_PROPERTIES, **properties}
    # java.util.Properties treats ':' as a key/value separator unless escaped
    lines = [f"{k}={v.replace(':', chr(92) + ':')}" for k, v in props.items()]
    (server_dir / "server.properties").write_text("\n".join(lines) + "\n")


def fixture_dimensions(spec: dict) -> list[str]:
    """`dimensions` (several, each rendered as its own map), else `dimension`, else the overworld."""
    return spec.get("dimensions") or [spec.get("dimension", OVERWORLD)]


def forceload_commands(x0: int, z0: int, x1: int, z1: int, dimension: str = OVERWORLD) -> list[str]:
    cx0, cz0, cx1, cz1 = x0 >> 4, z0 >> 4, x1 >> 4, z1 >> 4
    rows_per_batch = max(1, FORCELOAD_MAX_CHUNKS // (cx1 - cx0 + 1))
    out = []
    for cz in range(cz0, cz1 + 1, rows_per_batch):
        cz_end = min(cz + rows_per_batch - 1, cz1)
        out.append(f"execute in {dimension} run forceload add {cx0 * 16} {cz * 16} {cx1 * 16 + 15} {cz_end * 16 + 15}")
    return out


def wait_until_loaded(server: Server, x0: int, z0: int, x1: int, z1: int, dimension: str = OVERWORLD,
                      timeout: float = 600) -> None:
    deadline = time.monotonic() + timeout
    for x, z in [(x0, z0), (x0, z1), (x1, z0), (x1, z1)]:
        while True:
            m = server.query(f"execute in {dimension} if loaded {x} 0 {z}", r"Test (passed|failed)")
            if m.group(1) == "passed":
                break
            if time.monotonic() > deadline:
                raise ServerTimeout(f"chunk at {x},{z} never loaded")
            time.sleep(1)


def make_world(name: str, force: bool, tc: Toolchain = DEFAULT, heap: str = "4G", port: int | None = None) -> None:
    """`port` lets several servers run at once."""
    spec, commands = load_fixture(name)
    if port is not None:
        spec = {**spec, "properties": {**spec.get("properties", {}), "server-port": str(port)}}
    generate(tc.worlds / name, spec, commands, force, tc, heap)


def generate(server_dir, spec: dict, commands: list[str], force: bool, tc: Toolchain = DEFAULT, heap: str = "4G") -> None:
    """Fresh server in `server_dir`: properties + area from `spec`, then `commands`, save, stop."""
    if server_dir.exists():
        if not force:
            print(f"exists  {server_dir / 'world'} (use --force to regenerate)")
            return
        shutil.rmtree(server_dir)
    write_server_files(server_dir, spec.get("properties", {}))

    server = Server(server_dir, heap, tc)
    try:
        server.wait_for(r"Done \(", timeout=600, echo=True)
        area, dimension = spec["area"], spec.get("dimension", OVERWORLD)
        for dim in fixture_dimensions(spec):
            for cmd in forceload_commands(*area, dim):
                server.send(cmd)
            wait_until_loaded(server, *area, dim)
        print(f"loaded  area {area}")
        # patches (e.g. around known structures) are generated one at a time and released, so memory stays flat
        for extra in spec.get("extra_areas", []):
            for cmd in forceload_commands(*extra, dimension):
                server.send(cmd)
            wait_until_loaded(server, *extra, dimension)
            for cmd in forceload_commands(*extra, dimension):
                server.send(cmd.replace("forceload add", "forceload remove"))
        if spec.get("extra_areas"):
            print(f"loaded  {len(spec['extra_areas'])} extra areas")
        for cmd in commands:
            server.send(cmd)
        server.query("save-all flush", r"Saved the game", timeout=300)
    finally:
        code = server.stop()
    for err in server.errors:
        print(f"ERROR   {err}")
    print(f"world   {server_dir / 'world'} ({len(commands)} commands, {len(server.errors)} errors, exit {code})")
    if server.errors or code != 0:
        sys.exit(1)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("fixture")
    ap.add_argument("--force", action="store_true")
    ap.add_argument("--mc", default=DEFAULT.mc)
    ap.add_argument("--bluemap", default=DEFAULT.bluemap)
    args = ap.parse_args()
    from setup import resolve, setup  # lazy: only needed when a toolchain may need downloading

    tc = resolve(args.mc, args.bluemap)
    if tc != DEFAULT:
        setup(tc)
    make_world(args.fixture, args.force, tc)


if __name__ == "__main__":
    main()
