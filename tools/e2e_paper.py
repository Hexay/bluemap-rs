"""End-to-end test of the Paper plugin (docs/13) on a real Paper 26.3 server, plus an optional comparison run with
upstream BlueMap 5.28 on a copy of the same world.

    py -3 tools/e2e_paper.py [--skip-build | --core BIN | --jar JAR] [--keep] [--no-upstream]

Windows or Linux (host target from tools/build_core.py; on Linux the static musl core). Builds the core and the
plugin jars (or stages a prebuilt `--core`, or tests a given `--jar`), starts Paper with our jar + BlueBorder (marker addon) + server-side
bots, then checks: the core becomes ready, `/bluemap` commands answer on the console, the webserver serves the
webapp and a rendered tile, `live/players.json` lists a bot, BlueBorder's markers reach `live/markers.json`,
`/bluemap reload` works, a killed core is respawned, stopping the server leaves no core process, and neither does
SIGKILLing the JVM (stdin EOF). Results and
captured files go to work/e2e-paper/out/.
"""
import argparse
import json
import shutil
import sys
from pathlib import Path

from build_core import binary_name, build_jars, host_target
from build_core import build as build_core
from paper_server import E2E, Paper, fetch, fixture_world, get, json_get, kill, pid_alive, poll, prepare, rss_mib
from paths import ROOT

PLATFORM = ROOT / "platforms" / "paper"
TARGET = host_target()
OUT = E2E / "out"
RESULTS: list[tuple[str, bool, str]] = []


def check(name: str, ok, detail: str = "") -> bool:
    RESULTS.append((name, bool(ok), detail))
    print(f"[{'PASS' if ok else 'FAIL'}] {name} {detail}", flush=True)
    return bool(ok)


def build(core: Path | None, jobs: int | None) -> Path:
    """Stages the core (built here via tools/build_core.py, or a prebuilt `core`) and builds the plugin jars."""
    if core:
        native = PLATFORM / "natives" / TARGET / binary_name(TARGET)
        native.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(core, native)
        native.chmod(0o755)
    else:
        build_core(TARGET, jobs)
    build_jars()
    return plugin_jar()


def plugin_jar() -> Path:
    jars = sorted((PLATFORM / "build" / "libs").glob(f"*-{TARGET}.jar"))
    if not jars:
        sys.exit(f"no {TARGET} plugin jar in {PLATFORM / 'build' / 'libs'}; run without --skip-build")
    return jars[-1]


def clock(m) -> int:
    h, mi, se = map(int, m.group(1).split(":"))
    return h * 3600 + mi * 60 + se


def core_pid(server: Path) -> int | None:
    try:
        return int((server / "plugins" / "BlueMap" / ".core.pid").read_text().strip())
    except (OSError, ValueError):
        return None


def first_map() -> str | None:
    settings = json_get("settings.json")
    return settings["maps"][0] if settings and settings.get("maps") else None


def save(name: str, data: bytes) -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / name).write_bytes(data)


def run_ours(jar: Path, fresh: bool) -> Path:
    folder = E2E / "rs"
    # a world Paper generates itself keeps its spawn chunks unlit on disk for the first sessions, and both BlueMaps
    # skip unlit chunks; the `context` fixture (vanilla 26.3 server) is lit and small
    prepare(folder, [jar, fetch("blueborder"), fetch("bots")], fresh, world_from=fixture_world("context"))
    server = Paper(folder)
    try:
        spawned = server.wait_for(r"\[(\d+:\d+:\d+) .*BlueMap core \S+ started", 600, since_start=True)
        loaded = server.wait_for(r"\[(\d+:\d+:\d+) .*\[BlueMap\] Loaded!", 600, since_start=True)
        check("core ready", True, f"{clock(loaded) - clock(spawned)} s from core spawn to Ready")
        jar_mib = jar.stat().st_size / 2**20
        check("plugin jar", True, f"{jar.name} {jar_mib:.1f} MiB")
        server.wait_for(r"Done \(", 300, since_start=True)

        server.command("bluemap", r"BlueMap Status|render-threads", 30)
        check("/bluemap status", True)
        server.command("bluemap maps", r"BlueMap Maps", 30)
        check("/bluemap maps", True)

        status, index = get("index.html")
        check("webserver serves webapp", status == 200 and b"<html" in index.lower(), f"HTTP {status}")
        map_id = poll(first_map, 30)
        check("settings.json lists maps", map_id, str(map_id))

        players = poll(lambda: json_get(f"maps/{map_id}/live/players.json"), 15)
        check("players.json (no players)", players == {"players": []}, json.dumps(players))
        save("rs-players-empty.json", get(f"maps/{map_id}/live/players.json")[1])

        markers = poll(lambda: (m := json_get(f"maps/{map_id}/live/markers.json")) and "worldborder" in m and m, 60)
        check("BlueBorder markers in markers.json", markers, str(list(markers or {})))
        save("rs-markers.json", get(f"maps/{map_id}/live/markers.json")[1])

        server.command(f"bluemap force-update {map_id}", r"Created new update-task", 120)
        check("/bluemap force-update", True)
        tile = poll(lambda: get(f"maps/{map_id}/tiles/1/x0/z0.png")[0] == 200, 600, 5)
        check("map renders (lowres tile 1/x0/z0)", tile)
        # single-digit coordinates need no per-digit folders
        around = [f"maps/{map_id}/tiles/0/x{x}/z{z}.prbm" for x in range(-9, 10) for z in range(-9, 10)]
        hires = poll(lambda: next((p for p in around if get(p)[0] == 200), None), 300, 5)
        check("hires tile served", hires, str(hires))

        server.send("start 1 none")
        bot = poll(lambda: (p := json_get(f"maps/{map_id}/live/players.json")) and p["players"] and p, 60)
        check("players.json shows a bot", bot, json.dumps(bot)[:200] if bot else "")
        save("rs-players-bot.json", get(f"maps/{map_id}/live/players.json")[1])
        server.send("stress stop")

        pid = core_pid(folder)
        rss = rss_mib(pid) if pid else None
        check("core RSS", rss is not None, f"{rss:.0f} MiB" if rss else "")

        server.command("bluemap reload", r"BlueMap reloaded!", 300)
        check("/bluemap reload", True)
        again = poll(lambda: (m := json_get(f"maps/{map_id}/live/markers.json")) and "worldborder" in m, 60)
        check("markers back after reload", again)

        old = core_pid(folder)
        kill(old)
        new = poll(lambda: (p := core_pid(folder)) and p != old and pid_alive(p) and p, 60)
        check("killed core respawns", new, f"pid {old} -> {new}")
        back = poll(lambda: get("index.html")[0] == 200, 120)
        check("webserver back after respawn", back)
        check("markers after respawn",
              poll(lambda: (m := json_get(f"maps/{map_id}/live/markers.json")) and "worldborder" in m, 90))
    finally:
        last = core_pid(folder)
        code = server.stop()
        check("server stopped", code == 0, f"exit {code}")
        if last:
            check("no orphan core process", poll(lambda: not pid_alive(last), 30), f"pid {last}")
    return folder


def run_jvm_kill(folder: Path) -> None:
    """Restart on the same folder, SIGKILL the JVM: the core must exit by itself (stdin EOF / parent watchdog)."""
    server = Paper(folder)
    try:
        server.wait_for(r"\[BlueMap\] Loaded!", 600, since_start=True)
        pid = poll(lambda: (p := core_pid(folder)) and pid_alive(p) and p, 30)
    finally:
        server.kill()
    check("core exits after JVM SIGKILL", pid and poll(lambda: not pid_alive(pid), 30), f"pid {pid}")


def run_upstream(world: Path) -> None:
    folder = E2E / "upstream"
    prepare(folder, [fetch("upstream"), fetch("blueborder")], True, world_from=world)
    server = Paper(folder)
    try:
        server.wait_for(r"\[BlueMap\].*Loaded!", 600, since_start=True)
        server.wait_for(r"Done \(", 300, since_start=True)
        map_id = poll(first_map, 30)
        save("upstream-players-empty.json", get(f"maps/{map_id}/live/players.json")[1])
        poll(lambda: (m := json_get(f"maps/{map_id}/live/markers.json")) and "worldborder" in m, 60)
        save("upstream-markers.json", get(f"maps/{map_id}/live/markers.json")[1])
    finally:
        server.stop()
    for name in ["players-empty.json", "markers.json"]:
        ours, theirs = (OUT / f"rs-{name}").read_bytes(), (OUT / f"upstream-{name}").read_bytes()
        check(f"{name} byte-identical to upstream", ours == theirs, f"{len(ours)} vs {len(theirs)} B")
    compare_configs(E2E / "rs" / "plugins" / "BlueMap", folder / "plugins" / "BlueMap")


def compare_configs(ours: Path, theirs: Path) -> None:
    for path in sorted(theirs.rglob("*.conf")):
        rel = path.relative_to(theirs)
        mine = ours / rel
        if not mine.is_file():
            check(f"config {rel} generated", False)
            continue
        strip = lambda p: [l for l in p.read_text(encoding="utf-8").splitlines() if not l.startswith("# 20")]
        ours_lines, theirs_lines = strip(mine), strip(path)
        if rel.parts[0] == "storages":
            # deliberate: new storages get an appended `format: optimized` block (docs/00 "Storage default")
            check(f"config {rel} = upstream + format block", ours_lines[:len(theirs_lines)] == theirs_lines)
        else:
            check(f"config {rel} identical", ours_lines == theirs_lines)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--skip-build", action="store_true")
    ap.add_argument("--keep", action="store_true", help="reuse the server folder (world, configs) of the last run")
    ap.add_argument("--no-upstream", action="store_true")
    ap.add_argument("--core", type=Path, help="stage this prebuilt core binary instead of building one")
    ap.add_argument("--jar", type=Path, help="test this plugin jar (no build)")
    ap.add_argument("--jobs", "-j", type=int, help="cargo jobs")
    args = ap.parse_args()
    jar = args.jar or (plugin_jar() if args.skip_build else build(args.core, args.jobs))
    if OUT.exists():
        shutil.rmtree(OUT)
    folder = run_ours(jar, fresh=not args.keep)
    run_jvm_kill(folder)
    if not args.no_upstream:
        run_upstream(folder / "world")
    failed = [r for r in RESULTS if not r[1]]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} checks passed")
    (OUT / "results.json").write_text(json.dumps(RESULTS, indent=1))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
