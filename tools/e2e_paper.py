"""End-to-end test of the Paper plugin (docs/13) on a real Paper 26.3 server, plus an optional comparison run with
upstream BlueMap 5.28 on a copy of the same world.

    py -3 tools/e2e_paper.py [--skip-build | --core BIN | --jar JAR] [--keep] [--no-upstream] [--server folia] [--mc 26.2]

`--server`/`--mc` pick another build from e2e_server.SERVERS; other versions need their fixture world
(`py -3 tools/make_world.py context --mc <mc>`). Folia loads neither BlueBorder nor the bots (no `folia-supported`),
so its run skips the marker and player checks.

Windows or Linux (host target from tools/build_core.py; on Linux the static musl core). Builds the core and the
plugin jars (or stages a prebuilt `--core`, or tests a given `--jar`), starts Paper with our jar + BlueBorder (marker addon) + server-side
bots, then checks: the core becomes ready, `/bluemap` commands answer on the console, the webserver serves the
webapp and a rendered tile, `live/players.json` lists a bot, BlueBorder's markers reach `live/markers.json`,
`/bluemap reload` works, a killed core is respawned, stopping the server leaves no core process, and neither does
SIGKILLing the JVM (stdin EOF), and upstream resumes a forced render ours stopped partway (tasks.dat). Results and
captured files go to work/e2e-paper/out/.
"""
import argparse
import json
import re
import shutil
import sys
import time
from dataclasses import dataclass
from pathlib import Path

from build_core import build_jars, host_jar, stage_host_core
from e2e_server import (E2E, MC, SERVERS, Paper, check, fetch, fixture_world, get, json_get, kill, pid_alive, poll,
                        prepare, report, rss_mib)

OUT = E2E / "out"


@dataclass(frozen=True)
class Target:
    kind: str = "paper"
    mc: str = MC

    @property
    def addons(self) -> bool:
        """BlueBorder and the bots run on Paper only."""
        return self.kind == "paper"

    def server(self, folder: Path) -> Paper:
        return Paper(folder, kind=self.kind, mc=self.mc)

    def prepare(self, folder: Path, plugins: list[Path], fresh: bool, world_from: Path | None = None) -> None:
        prepare(folder, plugins, fresh, world_from=world_from, mc=self.mc)


def build(core: Path | None, jobs: int | None) -> Path:
    """Stages the core (built here via tools/build_core.py, or a prebuilt `core`) and builds the plugin jars."""
    stage_host_core(core, jobs)
    build_jars(("paper",))
    return plugin_jar()


def plugin_jar() -> Path:
    return host_jar("paper")


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


def run_ours(jar: Path, fresh: bool, target: Target) -> Path:
    folder = E2E / "rs"
    # a world Paper generates itself keeps its spawn chunks unlit on disk for the first sessions, and both BlueMaps
    # skip unlit chunks; the `context` fixture (vanilla server) is lit and small
    addons = [fetch("blueborder"), fetch("bots")] if target.addons else []
    target.prepare(folder, [jar, *addons], fresh, world_from=fixture_world("context", target.mc))
    server = target.server(folder)
    try:
        spawned = server.wait_for(r"\[(\d+:\d+:\d+) .*BlueMap core \S+ started", 600, since_start=True)
        loaded = server.wait_for(r"\[(\d+:\d+:\d+) .*\[BlueMap\] Loaded!", 600, since_start=True)
        check("core ready", True, f"{clock(loaded) - clock(spawned)} s from core spawn to Ready")
        jar_mib = jar.stat().st_size / 2**20
        check("plugin jar", True, f"{jar.name} {jar_mib:.1f} MiB")
        server.wait_for(r"Done \(", 300, since_start=True)

        server.command("bluemap", r"BlueMap Status|render-threads", 30)
        check("/bluemap status", True)
        if target.kind == "folia":
            # no global tick to average: the shim sends no ServerLoad and MSPT pausing stays off
            check("Folia detected", server.wait_for(r"Folia detected, enabling folia-support mode", 1, since_start=True))
        else:
            server.wait_for(r"Rendering pauses while the server's average tick time", 30, since_start=True)
            check("core receives ServerLoad (MSPT)", True)
        server.command("bluemap maps", r"BlueMap Maps", 30)
        check("/bluemap maps", True)

        status, index = get("index.html")
        check("webserver serves webapp", status == 200 and b"<html" in index.lower(), f"HTTP {status}")
        map_id = poll(first_map, 30)
        check("settings.json lists maps", map_id, str(map_id))

        players = poll(lambda: json_get(f"maps/{map_id}/live/players.json"), 15)
        check("players.json (no players)", players == {"players": []}, json.dumps(players))
        save("rs-players-empty.json", get(f"maps/{map_id}/live/players.json")[1])

        if target.addons:
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
        diagnostic_commands(server, folder, map_id)

        if target.addons:
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
        if target.addons:
            again = poll(lambda: (m := json_get(f"maps/{map_id}/live/markers.json")) and "worldborder" in m, 60)
            check("markers back after reload", again)

        old = core_pid(folder)
        kill(old)
        new = poll(lambda: (p := core_pid(folder)) and p != old and pid_alive(p) and p, 60)
        check("killed core respawns", new, f"pid {old} -> {new}")
        back = poll(lambda: get("index.html")[0] == 200, 120)
        check("webserver back after respawn", back)
        if target.addons:
            check("markers after respawn",
                  poll(lambda: (m := json_get(f"maps/{map_id}/live/markers.json")) and "worldborder" in m, 90))
        save("rs-frozen.json", json.dumps(frozen_texts(server), indent=1, ensure_ascii=False).encode())
    finally:
        last = core_pid(folder)
        code = server.stop()
        check("server stopped", code == 0, f"exit {code}")
        if last:
            check("no orphan core process", poll(lambda: not pid_alive(last), 30), f"pid {last}")
    check("tasks.dat written on stop", any((folder / "bluemap").rglob("tasks.dat")))
    return folder


def diagnostic_commands(server: Paper, folder: Path, map_id: str) -> None:
    """troubleshoot / debug / storages from the console (no sender world, so only the explicit-argument forms)."""
    server.command("bluemap troubleshoot", r"Troubleshooting", 30)
    check("/bluemap troubleshoot", True)
    server.command(f"bluemap troubleshoot {map_id} 0 0", r"Troubleshooting", 30)
    check("/bluemap troubleshoot <map> <x> <z>", True)
    server.command(f"bluemap debug world {map_id} 0 64 0", r"World-Info \(debug\)", 30)
    check("/bluemap debug world <map> <x> <y> <z>", server.wait_for(r"block: minecraft:", 10))
    server.command(f"bluemap debug map {map_id} 0 0", r"Map-Info \(debug\)", 30)
    check("/bluemap debug map <map> <x> <z>", server.wait_for(r"state: ", 10))
    server.command("bluemap debug dump", r"created at: ", 30)
    check("/bluemap debug dump", any(folder.rglob("dump.json")))
    texts = command_texts(server, map_id)
    check("/bluemap storages", texts["bluemap storages"])
    check("/bluemap storages <storage>", any("Type: bluemap:file" in l for l in texts["bluemap storages file"]))
    save("rs-commands.json", json.dumps(texts, indent=1, ensure_ascii=False).encode())


# console output that only depends on configs and the world on disk, compared verbatim with upstream
STABLE_COMMANDS = [("bluemap storages", r"BlueMap Storages"), ("bluemap storages file", r"BlueMap Storage 'file'"),
                   ("bluemap debug world {map} 0 64 0", r"World-Info \(debug\)")]


def command_texts(server: Paper, map_id: str) -> dict[str, list[str]]:
    return {cmd: server.command_text(cmd.format(map=map_id), header, 30) for cmd, header in STABLE_COMMANDS}


def frozen_texts(server: Paper) -> dict[str, list[str]]:
    """`/bluemap` and `/bluemap maps` with every map frozen and the render-threads stopped: the queue is empty, so
    both texts only depend on the maps. Unfreezes and restarts afterwards (a `--keep` run reuses the state)."""
    maps = json_get("settings.json")["maps"]
    for map_id in maps:
        server.command(f"bluemap freeze {map_id}", r"is (now|already) frozen", 30)
    time.sleep(3)  # upstream drops a cancelled running task only when its render thread next looks at it
    server.command("bluemap stop", r"Render-Threads are (now|already) stopped", 30)
    texts = {cmd: server.command_text(cmd, header, 30) for cmd, header in [("bluemap", r"BlueMap Status"),
                                                                           ("bluemap maps", r"BlueMap Maps")]}
    for map_id in maps:
        server.command(f"bluemap unfreeze {map_id}", r"no longer frozen|is not frozen", 30)
    server.command("bluemap start", r"Render-Threads are (now|already) running", 30)
    return texts


def compare_texts(name: str) -> None:
    ours, theirs = (json.loads((OUT / f"{side}-{name}").read_text(encoding="utf-8")) for side in ("rs", "upstream"))
    for cmd, lines in ours.items():
        other = theirs[cmd]
        diff = [f"{a!r} vs {b!r}" for a, b in zip(lines, other) if a != b] or len(lines) != len(other)
        check(f"/{cmd.replace('{map}', '<map>')} text identical to upstream", not diff, str(diff)[:300] if diff else "")


def run_jvm_kill(folder: Path, target: Target) -> None:
    """Restart on the same folder, SIGKILL the JVM: the core must exit by itself (stdin EOF / parent watchdog)."""
    server = target.server(folder)
    try:
        server.wait_for(r"\[BlueMap\] Loaded!", 600, since_start=True)
        pid = poll(lambda: (p := core_pid(folder)) and pid_alive(p) and p, 30)
    finally:
        server.kill()
    check("core exits after JVM SIGKILL", pid and poll(lambda: not pid_alive(pid), 30), f"pid {pid}")


def run_handoff(jar: Path, rs: Path) -> None:
    """Switching back: ours stops a forced render of `structures` partway (tasks.dat with done regions), then upstream
    loads that folder and must resume the task where ours left off, not from 0 and not fail on tasks.dat."""
    folder = E2E / "handoff"
    if folder.exists():
        shutil.rmtree(folder)
    shutil.copytree(rs, folder, ignore=shutil.ignore_patterns("world", "session.lock", "tasks.dat", "*.log*"))
    shutil.rmtree(folder / "bluemap" / "web" / "maps", ignore_errors=True)
    core_conf = folder / "plugins" / "BlueMap" / "core.conf"
    core_conf.write_text(re.sub(r"(?m)^render-thread-count: .*$", "render-thread-count: 1",
                                core_conf.read_text(encoding="utf-8")), encoding="utf-8")
    prepare(folder, [jar], False, world_from=fixture_world("structures"))
    server = Paper(folder)
    try:
        server.wait_for(r"\[BlueMap\] Loaded!", 600, since_start=True)
        map_id = poll(first_map, 30)
        server.command("bluemap stop", r"Render-Threads are (now|already) stopped", 30)
        server.command(f"bluemap force-update {map_id}", r"Created new update-task", 60)
        server.command("bluemap start", r"Render-Threads are (now|already) running", 30)
        ours = poll(lambda: (p := status_progress(server)) and p > 0 and p, 120, 0.2)
        server.command("bluemap stop", r"Render-Threads are (now|already) stopped", 30)
    finally:
        server.stop()
    check("tasks.dat with a partly done task", ours and (folder / "bluemap" / "tasks.dat").is_file(), f"{ours}%")

    prepare(folder, [fetch("upstream")], False)
    server = Paper(folder)
    try:
        server.wait_for(r"\[BlueMap\].*Loaded!", 600, since_start=True)
        failed = re.search(r"Failed to load tasks\.dat", "\n".join(server.history))
        line = " ".join(server.command_text("bluemap maps", r"BlueMap Maps|Maps", 30))
    finally:
        server.stop()
    theirs = re.search(rf"{re.escape(map_id)}.*?being updated: ([\d.]+)%", line)
    check("upstream reads our tasks.dat", not failed and theirs, line[:300])
    check("upstream resumes past our done regions", theirs and float(theirs.group(1)) > 0,
          f"ours {ours}% at stop, upstream {theirs.group(1) if theirs else '?'}% before rendering")


def status_progress(server: Paper) -> float | None:
    m = server.command("bluemap", r"progress: ([\d.]+)%|render-threads are (idle|paused)", 10)
    return float(m.group(1)) if m.group(1) else None


def run_upstream(world: Path, target: Target) -> None:
    folder = E2E / "upstream"
    target.prepare(folder, [fetch("upstream"), *([fetch("blueborder")] if target.addons else [])], True, world_from=world)
    server = target.server(folder)
    try:
        server.wait_for(r"\[BlueMap\].*Loaded!", 600, since_start=True)
        server.wait_for(r"Done \(", 300, since_start=True)
        map_id = poll(first_map, 30)
        save("upstream-players-empty.json", get(f"maps/{map_id}/live/players.json")[1])
        if target.addons:
            poll(lambda: (m := json_get(f"maps/{map_id}/live/markers.json")) and "worldborder" in m, 60)
            save("upstream-markers.json", get(f"maps/{map_id}/live/markers.json")[1])
        texts = command_texts(server, map_id)
        save("upstream-commands.json", json.dumps(texts, indent=1, ensure_ascii=False).encode())
        save("upstream-frozen.json", json.dumps(frozen_texts(server), indent=1, ensure_ascii=False).encode())
    finally:
        server.stop()
    for name in ["players-empty.json", *(["markers.json"] if target.addons else [])]:
        ours, theirs = (OUT / f"rs-{name}").read_bytes(), (OUT / f"upstream-{name}").read_bytes()
        check(f"{name} byte-identical to upstream", ours == theirs, f"{len(ours)} vs {len(theirs)} B")
    compare_texts("commands.json")
    compare_texts("frozen.json")
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
    ap.add_argument("--server", choices=sorted({kind for kind, _ in SERVERS}), default="paper")
    ap.add_argument("--mc", default=MC)
    args = ap.parse_args()
    target = Target(args.server, args.mc)
    if (target.kind, target.mc) not in SERVERS:
        ap.error(f"no pinned {target.kind} build for {target.mc}; known: {sorted(SERVERS)}")
    jar = args.jar or (plugin_jar() if args.skip_build else build(args.core, args.jobs))
    if OUT.exists():
        shutil.rmtree(OUT)
    folder = run_ours(jar, not args.keep, target)
    run_jvm_kill(folder, target)
    if not args.no_upstream:
        run_upstream(folder / "world", target)
        if target == Target():
            run_handoff(jar, folder)
    sys.exit(report(OUT))


if __name__ == "__main__":
    main()
