"""End-to-end test of the Fabric mod (docs/16) on a real Fabric 26.3 dedicated server.

    py -3 tools/e2e_fabric.py [--skip-build | --core BIN | --jar JAR] [--keep]

Windows or Linux. Builds the core and the mod jars (or stages a prebuilt `--core`, or tests a given `--jar`), installs
Fabric Loader into work/e2e-paper/cache (Fabric's installer; the vanilla jar from work/downloads), starts the server with
our jar + Fabric API + Carpet (fake players) + BlueMap Offline Player Markers (a Fabric BlueMapAPI addon), then checks:
the core becomes ready, `/bluemap` commands answer on the console, the addon gets BlueMapAPI.onEnable and registers its
script, the webserver serves the webapp and rendered tiles, `live/players.json` lists a Carpet bot and drops it again, the
(non-op) bot may run `/bluemap stop` only once LuckPerms grants the node,
`/bluemap reload` works, a killed core is respawned, stopping the server leaves no core process, neither does killing
the JVM, and next to upstream BlueMap's Fabric jar at most one BlueMap runs. Results go to work/e2e-fabric/out/.
"""
import argparse
import json
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

from build_core import build_jars, host_jar, stage_host_core
from e2e_server import (CACHE, JAVA, MC, Server, check, fetch, fixture_world, get, json_get, kill, pid_alive, poll,
                        prepare, report, rss_mib)
from paths import DEFAULT, WORK

LOADER = "0.19.5"
E2E = WORK / "e2e-fabric"
OUT = E2E / "out"
CLOCK = re.compile(r"\[(\d+):(\d+):(\d+)")


def build(core: Path | None, jobs: int | None) -> Path:
    stage_host_core(core, jobs)
    build_jars(("fabric",))
    return host_jar("fabric")


def fabric_launcher() -> Path:
    """fabric-server-launch.jar (+ libraries/) for MC 26.3, installed once into the cache."""
    home = CACHE / f"fabric-{MC}-{LOADER}"
    launcher = home / "fabric-server-launch.jar"
    if not launcher.is_file():
        home.mkdir(parents=True, exist_ok=True)
        subprocess.run([str(JAVA), "-jar", str(fetch("fabric-installer")), "server", "-dir", str(home),
                        "-mcversion", MC, "-loader", LOADER], check=True)
    return launcher


class Fabric(Server):
    def __init__(self, folder: Path):
        # the launcher's manifest class path is relative to its own folder; the vanilla jar is shared
        (folder / "fabric-server-launcher.properties").write_text(f"serverJar={DEFAULT.server_jar.as_posix()}\n")
        super().__init__(folder, fabric_launcher())


def secs(m: re.Match) -> int:
    h, mi, se = map(int, CLOCK.search(m.string).groups())
    return h * 3600 + mi * 60 + se


def core_pid(server: Path) -> int | None:
    try:
        return int((server / "config" / "bluemap" / ".core.pid").read_text().strip())
    except (OSError, ValueError):
        return None


def first_map() -> str | None:
    settings = json_get("settings.json")
    return settings["maps"][0] if settings and settings.get("maps") else None


def live(map_id: str, name: str):
    return json_get(f"maps/{map_id}/live/{name}.json")


def run_ours(jar: Path, fresh: bool) -> Path:
    folder = E2E / "rs"
    mods = [jar, fetch("fabric-api"), fetch("carpet"), fetch("offline-markers"), fetch("luckperms-fabric")]
    prepare(folder, mods, fresh, world_from=fixture_world("context"), addons_dir="mods")
    server = Fabric(folder)
    try:
        spawned = server.wait_for(r"BlueMap core \S+ started", 600, since_start=True)
        loaded = server.wait_for(r"\bLoaded!", 600, since_start=True)
        check("core ready", True, f"{secs(loaded) - secs(spawned)} s from core spawn to Ready")
        check("mod jar", True, f"{jar.name} {jar.stat().st_size / 2**20:.1f} MiB")
        server.wait_for(r"Done \(", 300, since_start=True)

        server.command("bluemap", r"BlueMap Status|render-threads", 30)
        check("/bluemap status", True)
        server.wait_for(r"Rendering pauses while the server's average tick time", 30, since_start=True)
        check("core receives ServerLoad (MSPT)", True)
        server.command("bluemap version", r"bluemap-rs Fabric shim 5\.28\+rs\.", 30)
        check("/bluemap version names the Fabric shim", True)
        server.command("bluemap maps", r"BlueMap Maps", 30)
        check("/bluemap maps", True)

        status, index = get("index.html")
        check("webserver serves webapp", status == 200 and b"<html" in index.lower(), f"HTTP {status}")
        map_id = poll(first_map, 30)
        check("settings.json lists maps", map_id, str(map_id))
        check("players.json (no players)", poll(lambda: live(map_id, "players") == {"players": []}, 15),
              json.dumps(live(map_id, "players")))
        addon(server)

        server.command(f"bluemap force-update {map_id}", r"Created new update-task", 120)
        check("/bluemap force-update", True)
        check("map renders (lowres tile 1/x0/z0)",
              poll(lambda: get(f"maps/{map_id}/tiles/1/x0/z0.png")[0] == 200, 600, 5))
        around = [f"maps/{map_id}/tiles/0/x{x}/z{z}.prbm" for x in range(-9, 10) for z in range(-9, 10)]
        hires = poll(lambda: next((p for p in around if get(p)[0] == 200), None), 300, 5)
        check("hires tile served", hires, str(hires))

        players(server, map_id)

        pid = core_pid(folder)
        rss = rss_mib(pid) if pid else None
        check("core RSS", rss is not None, f"{rss:.0f} MiB" if rss else "")

        server.command("bluemap reload", r"BlueMap reloaded!", 300)
        check("/bluemap reload", True)

        old = core_pid(folder)
        kill(old)
        new = poll(lambda: (p := core_pid(folder)) and p != old and pid_alive(p) and p, 60)
        check("killed core respawns", new, f"pid {old} -> {new}")
        check("webserver back after respawn", poll(lambda: get("index.html")[0] == 200, 120))
    finally:
        last = core_pid(folder)
        code = server.stop()
        check("server stopped", code == 0, f"exit {code}")
        if last:
            check("no orphan core process", poll(lambda: not pid_alive(last), 30), f"pid {last}")
    check("tasks.dat written on stop", any((folder / "bluemap").rglob("tasks.dat")))
    return folder


def players(server: Server, map_id: str) -> None:
    """A Carpet bot joins (players.json) and leaves (players.json empty again)."""
    server.command("player e2ebot spawn", r"e2ebot joined the game", 60)
    bot = poll(lambda: (p := live(map_id, "players")) and p["players"] and p, 30)
    check("players.json shows a bot", bot and bot["players"][0]["name"] == "e2ebot", json.dumps(bot)[:200])
    (OUT / "rs-players-bot.json").write_text(json.dumps(bot))
    permissions(server)
    server.command("player e2ebot kill", r"e2ebot left the game", 60)
    check("players.json drops the bot", poll(lambda: live(map_id, "players") == {"players": []}, 15))


def permissions(server: Server) -> None:
    """The bot is no op: `/bluemap stop` run as it must do nothing until LuckPerms grants it bluemap.stop."""
    server.send("execute as e2ebot run bluemap stop")
    time.sleep(2)  # the core answers asynchronously; nothing on the console to wait for when it refuses
    check("non-op player without the node is refused", not threads_stopped(server))
    server.command("lp user e2ebot permission set bluemap.stop true", r"(?i)set .*bluemap\.stop.* to true", 60)
    server.send("execute as e2ebot run bluemap stop")
    check("LuckPerms node grants the command", poll(lambda: threads_stopped(server), 15))
    server.command("bluemap start", r"Render-Threads started", 30)


def threads_stopped(server: Server) -> bool:
    m = server.command("bluemap", r"render-threads are (\w+)|progress: ", 10)
    return m.group(1) == "stopped"


def addon(server: Server) -> None:
    """Offline Player Markers (depends on mod id `bluemap`) ran its BlueMapAPI.onEnable and registered its script."""
    server.wait_for(r"API Ready! BlueMap Offline Player Markers", 30, since_start=True)
    scripts = poll(lambda: (s := json_get("settings.json")) and s.get("scripts"), 30)
    check("Fabric addon on BlueMapAPI (onEnable, registerScript)", scripts and any("bmopm" in s for s in scripts),
          str(scripts))


def run_jvm_kill(folder: Path) -> None:
    """Restart on the same folder, kill the JVM: the core must exit by itself (stdin EOF / parent watchdog)."""
    server = Fabric(folder)
    try:
        server.wait_for(r"\bLoaded!", 600, since_start=True)
        pid = poll(lambda: (p := core_pid(folder)) and pid_alive(p) and p, 30)
    finally:
        server.kill()
    check("core exits after JVM kill", pid and poll(lambda: not pid_alive(pid), 30), f"pid {pid}")


def run_duplicate(jar: Path) -> None:
    """Upstream's Fabric jar next to ours: both are mod id `bluemap` and Fabric Loader silently loads one of them.
    Either upstream runs alone, or ours stands down (Coexistence) — never both on the same maps."""
    folder = E2E / "duplicate"
    prepare(folder, [jar, fetch("upstream-fabric"), fetch("fabric-api")], True, addons_dir="mods")
    server = Fabric(folder)
    try:
        m = server.wait_for(r"BlueMap loaded!|bluemap-rs stays inactive", 300, since_start=True)
        server.wait_for(r"Done \(", 300, since_start=True)
        winner = "upstream" if "loaded" in m.group(0) else "bluemap-rs (stood down)"
    finally:
        code = server.stop()
    spawned = (folder / "config" / "bluemap" / ".core.pid").exists()
    check("upstream Fabric jar next to ours: one BlueMap at most", not spawned, f"loader picked {winner}; exit {code}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--skip-build", action="store_true")
    ap.add_argument("--keep", action="store_true", help="reuse the server folder (world, configs) of the last run")
    ap.add_argument("--core", type=Path, help="stage this prebuilt core binary instead of building one")
    ap.add_argument("--jar", type=Path, help="test this mod jar (no build)")
    ap.add_argument("--jobs", "-j", type=int, help="cargo jobs")
    args = ap.parse_args()
    jar = args.jar or (host_jar("fabric") if args.skip_build else build(args.core, args.jobs))
    if OUT.exists():
        shutil.rmtree(OUT)
    OUT.mkdir(parents=True)
    folder = run_ours(jar, fresh=not args.keep)
    run_jvm_kill(folder)
    run_duplicate(jar)
    sys.exit(report(OUT))


if __name__ == "__main__":
    main()
