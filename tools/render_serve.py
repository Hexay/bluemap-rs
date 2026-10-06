"""Render a fixture world with BlueMap CLI and/or serve it on WEB_HOST:WEB_PORT.

Usage: py -3 tools/render_serve.py <fixture> [--no-render] [--no-serve] [--force-render] [--mc 1.21.11]
                                   [--world <dir> --name <out-name> [--relight]] [--port N]
Layout: <toolchain>/bluemap/<name>/{config,data,web}; name defaults to the fixture; map id = fixture name.
Map settings are BlueMap defaults (what public maps run) unless fixture.json has a "bluemap" object.
--world renders another world (e.g. a reconstruction) with the fixture's map config; give it its own --name so
it gets its own webroot. Map ids stay the fixture's, so a camera URL hash works on both sites.
--relight first resaves --world in the server over the fixture area (each of its dimensions): worlds written without light data, and BlueMap
skips/darkens unlit chunks.
"""
import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

from make_world import fixture_dimensions
from paths import DEFAULT, FIXTURES, WEB_HOST, WEB_PORT, Toolchain

TEMPLATES = {"minecraft:overworld": "overworld.conf", "minecraft:the_nether": "nether.conf", "minecraft:the_end": "end.conf"}


def set_conf(path: Path, key: str, value: str) -> None:
    text = path.read_text()
    line = f"{key}: {value}"
    new, n = re.subn(rf"^{re.escape(key)}:.*$", line, text, flags=re.M)
    path.write_text(new if n else text.rstrip() + "\n" + line + "\n")


def conf_value(v) -> str:
    return json.dumps(v) if isinstance(v, str) else str(v).lower() if isinstance(v, bool) else str(v)


def bluemap(cwd: Path, *args: str, tc: Toolchain = DEFAULT) -> subprocess.Popen:
    cmd = [str(tc.bluemap_java), "-jar", str(tc.bluemap_jar), "-c", "config", "-v", tc.mc, *args]
    return subprocess.Popen(cmd, cwd=cwd)


def configure(fixture: str, tc: Toolchain = DEFAULT, world: Path | None = None, name: str | None = None,
              port: int = WEB_PORT) -> Path:
    """BlueMap dir <bluemap_root>/<name or fixture> rendering `world` (default: the fixture's) with the
    fixture's map config, webserver on `port`. Returns the BlueMap dir."""
    world = Path(world).resolve() if world else tc.worlds / fixture / "world"
    if not world.exists():
        sys.exit(f"no world at {world}; run make_world.py {fixture} first")
    base = tc.bluemap_root / (name or fixture)
    cfg = base / "config"
    if not (cfg / "core.conf").exists():
        base.mkdir(parents=True, exist_ok=True)
        bluemap(base, tc=tc).wait()
    set_conf(cfg / "core.conf", "accept-download", "true")
    set_conf(cfg / "core.conf", "metrics", "false")
    set_conf(cfg / "core.conf", "render-thread-count", str(os.cpu_count() or 1))
    set_conf(cfg / "webserver.conf", "ip", json.dumps(WEB_HOST))
    set_conf(cfg / "webserver.conf", "port", str(port))

    spec = json.loads((FIXTURES / fixture / "fixture.json").read_text())
    maps = cfg / "maps"
    # BlueMap's own default map per dimension (the nether one masks out the roof, y 90..127). Several
    # dimensions keep BlueMap's default ids (overworld/nether/end), like a stock install; one is renamed.
    dims = fixture_dimensions(spec)
    targets = [maps / TEMPLATES[d] for d in dims] if len(dims) > 1 else [maps / f"{fixture}.conf"]
    if len(dims) == 1 and (maps / TEMPLATES[dims[0]]).exists():
        (maps / TEMPLATES[dims[0]]).replace(targets[0])
    for other in maps.glob("*.conf"):
        if other not in targets:
            other.unlink()
    for target in targets:
        set_conf(target, "world", json.dumps(world.as_posix()))
        if len(dims) == 1:
            set_conf(target, "name", json.dumps(fixture))
        for key, value in spec.get("bluemap", {}).items():
            set_conf(target, key, conf_value(value))
    return base


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("fixture")
    ap.add_argument("--no-render", action="store_true")
    ap.add_argument("--no-serve", action="store_true")
    ap.add_argument("--force-render", action="store_true")
    ap.add_argument("--world", type=Path, help="render this world instead of the fixture's (needs --name)")
    ap.add_argument("--name", help="BlueMap dir / webroot name (default: the fixture)")
    ap.add_argument("--relight", action="store_true", help="resave --world in the server first (computes light)")
    ap.add_argument("--port", type=int, default=WEB_PORT)
    ap.add_argument("--mc", default=DEFAULT.mc)
    ap.add_argument("--bluemap", default=DEFAULT.bluemap)
    args = ap.parse_args()
    if args.world and not args.name:
        ap.error("--world needs --name, or it would overwrite the fixture's own render")
    from setup import resolve  # lazy: network lookup only for non-default toolchains

    tc = resolve(args.mc, args.bluemap)
    if args.relight:
        if not args.world:
            ap.error("--relight needs --world")
        from resave_world import resave

        spec = json.loads((FIXTURES / args.fixture / "fixture.json").read_text())
        if resave(args.world.resolve(), spec["area"], tc, fixture_dimensions(spec)):
            sys.exit("relight failed")
    base = configure(args.fixture, tc, args.world, args.name, args.port)
    flags = []
    if not args.no_render:
        flags.append("-r")
        if args.force_render:
            flags.append("-f")
    if not args.no_serve:
        flags.append("-w")
        print(f"serving http://{WEB_HOST}:{args.port}/ (Ctrl+C to stop)", flush=True)
    if not flags:
        return
    proc = bluemap(base, *flags, tc=tc)
    try:
        sys.exit(proc.wait())
    except KeyboardInterrupt:
        proc.terminate()


if __name__ == "__main__":
    main()
