"""Open an existing world in a headless server of the given version, force-load an area, save, stop.
Proves the server accepts our chunks (a regenerated chunk would come back as template void).

Usage: py -3 tools/resave_world.py <world_dir> --area=x0,z0,x1,z1 [--mc 1.21.11]
The server runs in the world's parent dir with level-name = the world folder name.
"""
import argparse
import sys
from pathlib import Path

from console import Server
from make_world import BASE_PROPERTIES, OVERWORLD, forceload_commands, wait_until_loaded
from paths import DEFAULT, Toolchain


REGION_DIRS = {
    "minecraft:overworld": ["dimensions/minecraft/overworld/region", "region"],
    "minecraft:the_nether": ["dimensions/minecraft/the_nether/region", "DIM-1/region"],
    "minecraft:the_end": ["dimensions/minecraft/the_end/region", "DIM1/region"],
}


def written_area(world: Path, dimension: str = OVERWORLD) -> list[int] | None:
    """Block bounds x0,z0,x1,z1 of the chunks present in the world's region files for `dimension`."""
    chunks = []
    for rel in REGION_DIRS[dimension]:
        for f in (world / rel).glob("r.*.*.mca"):
            rx, rz = map(int, f.name.split(".")[1:3])
            header = f.read_bytes()[:4096]
            chunks += [(rx * 32 + i % 32, rz * 32 + i // 32) for i in range(1024) if header[4 * i:4 * i + 4] != b"\0" * 4]
    if not chunks:
        return None
    xs, zs = [c[0] for c in chunks], [c[1] for c in chunks]
    return [min(xs) * 16, min(zs) * 16, max(xs) * 16 + 15, max(zs) * 16 + 15]


def resave(world: Path, area: list[int], tc: Toolchain = DEFAULT, dimensions: list[str] = (OVERWORLD,)) -> int:
    """Force-loads `area` in each of `dimensions`."""
    server_dir = world.parent
    (server_dir / "eula.txt").write_text("eula=true\n")
    props = {**BASE_PROPERTIES, "level-name": world.name}
    (server_dir / "server.properties").write_text("".join(f"{k}={v}\n" for k, v in props.items()))
    server = Server(server_dir, tc=tc)
    try:
        server.wait_for(r"Done \(", timeout=600)
        for dim in dimensions:
            for cmd in forceload_commands(*area, dim):
                server.send(cmd)
            wait_until_loaded(server, *area, dim)
        server.query("save-all flush", r"Saved the game", timeout=300)
    finally:
        code = server.stop()
    for err in server.errors:
        print(f"ERROR   {err}")
    print(f"resaved {world} ({len(server.errors)} errors, exit {code})")
    return 1 if server.errors or code else 0


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("world", type=Path)
    # an option, not positional: argparse reads a leading "-96" as a flag (pass --area=-96,...)
    ap.add_argument("--area", required=True, help="x0,z0,x1,z1 block coords to force-load")
    ap.add_argument("--mc", default=DEFAULT.mc)
    ap.add_argument("--bluemap", default=DEFAULT.bluemap)
    args = ap.parse_args()
    from setup import resolve  # lazy: network lookup only for non-default toolchains

    tc = resolve(args.mc, args.bluemap)
    sys.exit(resave(args.world.resolve(), [int(v) for v in args.area.split(",")], tc))


if __name__ == "__main__":
    main()
