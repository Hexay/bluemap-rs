"""Headless Paper/Fabric servers for the shim e2e tests (e2e_paper.py, e2e_fabric.py): pinned downloads, server folder
setup, console + HTTP helpers, check results."""
import hashlib
import json
import re
import shutil
import subprocess
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

from paths import WINDOWS, WORK, jdk_dir

E2E = WORK / "e2e-paper"
CACHE = E2E / "cache"
JAVA = jdk_dir(25) / "bin" / ("java.exe" if WINDOWS else "java")
MC = "26.3"
HTTP = "http://127.0.0.1:8100"
ANSI = re.compile(r"\x1b\[[0-9;]*m")

# (file name, url, sha256 or None)
DOWNLOADS = {
    "paper": ("paper-26.3-159.jar",
              "https://fill-data.papermc.io/v1/objects/2224a0b2b6b096ff4c429ad926e97977213e4f633e90cb3a49b5eeb82f94bab0/paper-26.3-159.jar",
              "2224a0b2b6b096ff4c429ad926e97977213e4f633e90cb3a49b5eeb82f94bab0"),
    # marker-only addon from docs/13's survey: one world-border shape per world, zero config
    "blueborder": ("BlueBorder-1.1.2.jar",
                   "https://github.com/pop4959/BlueBorder/releases/download/1.1.2/BlueBorder-1.1.2.jar", None),
    # server-side fake players that show up in getOnlinePlayers()
    "bots": ("BetterStresstestbots-2.0.0-26.3.jar",
             "https://cdn.modrinth.com/data/DQNKpytv/versions/AMkho9sK/BetterStresstestbots-2.0.0-26.3.jar", None),
    "upstream": ("bluemap-5.28-paper.jar",
                 "https://cdn.modrinth.com/data/swbUV1cr/versions/pILlMIlN/bluemap-5.28-paper.jar", None),
    "fabric-installer": ("fabric-installer-1.1.2.jar",
                         "https://maven.fabricmc.net/net/fabricmc/fabric-installer/1.1.2/fabric-installer-1.1.2.jar",
                         None),
    "fabric-api": ("fabric-api-0.162.0+26.3.jar",
                   "https://cdn.modrinth.com/data/P7dR8mSH/versions/v2j28coa/fabric-api-0.162.0%2B26.3.jar", None),
    # server-side fake players: /player <name> spawn
    "carpet": ("fabric-carpet-26.3+v260915.jar",
               "https://cdn.modrinth.com/data/TQTTVgYE/versions/yt9oDFOj/fabric-carpet-26.3%2Bv260915.jar", None),
    # Fabric BlueMapAPI addon (depends on mod id `bluemap`): a marker per player that logged off
    "offline-markers": ("bluemap-offline-player-markers-2026.9.1.jar",
                        "https://cdn.modrinth.com/data/4h9u0qdE/versions/safOEocx/bluemap-offline-player-markers-2026.9.1.jar",
                        None),
    "upstream-fabric": ("bluemap-5.28-fabric.jar",
                        "https://cdn.modrinth.com/data/swbUV1cr/versions/bbzcTCOs/bluemap-5.28-fabric.jar", None),
}

RESULTS: list[tuple[str, bool, str]] = []

PROPERTIES = {
    "online-mode": "false", "server-port": "25590", "allow-flight": "true", "view-distance": "4",
    "simulation-distance": "4", "level-seed": "bluemap-e2e", "spawn-protection": "0", "motd": "bluemap e2e",
}


def check(name: str, ok, detail: str = "") -> bool:
    RESULTS.append((name, bool(ok), detail))
    print(f"[{'PASS' if ok else 'FAIL'}] {name} {detail}", flush=True)
    return bool(ok)


def report(out: Path) -> int:
    """Prints the summary, writes `out/results.json`; the exit code."""
    failed = [r for r in RESULTS if not r[1]]
    print(f"\n{len(RESULTS) - len(failed)}/{len(RESULTS)} checks passed")
    out.mkdir(parents=True, exist_ok=True)
    (out / "results.json").write_text(json.dumps(RESULTS, indent=1))
    return 1 if failed else 0


def fetch(key: str) -> Path:
    name, url, sha = DOWNLOADS[key]
    path = CACHE / name
    if not path.is_file():
        CACHE.mkdir(parents=True, exist_ok=True)
        print(f"downloading {name}", flush=True)
        tmp = path.with_suffix(".part")
        # Modrinth's CDN refuses Python's default user agent
        req = urllib.request.Request(url, headers={"User-Agent": "bluemap-rs-e2e/0.1"})
        with urllib.request.urlopen(req, timeout=120) as r, open(tmp, "wb") as f:
            shutil.copyfileobj(r, f)
        tmp.replace(path)
    if sha and hashlib.sha256(path.read_bytes()).hexdigest() != sha:
        raise RuntimeError(f"checksum mismatch: {path}")
    return path


def _work_dirs() -> list[Path]:
    # work/ is not versioned, so a worktree finds the fixtures in an enclosing checkout
    return [WORK, *[d / "work" for d in WORK.parent.parents]]


def client_jar() -> Path | None:
    """A cached 26.3 client jar, so neither BlueMap has to download it."""
    for work in _work_dirs():
        for p in (work / "bluemap").glob(f"*/data/minecraft-client-{MC}.jar"):
            return p
    return None


def fixture_world(name: str) -> Path:
    """A world made by tools/make_world.py (vanilla 26.3 server)."""
    for work in _work_dirs():
        if (world := work / "worlds" / name / "world").is_dir():
            return world
    raise FileNotFoundError(f"fixture world '{name}' missing: run py -3 tools/make_world.py {name}")


def prepare(folder: Path, plugins: list[Path], fresh: bool, world_from: Path | None = None,
            addons_dir: str = "plugins") -> None:
    """A server folder with exactly `plugins` in `addons_dir` (`mods` on Fabric)."""
    if fresh and folder.exists():
        shutil.rmtree(folder)
    addons = folder / addons_dir
    addons.mkdir(parents=True, exist_ok=True)
    (folder / "eula.txt").write_text("eula=true\n")
    (folder / "server.properties").write_text("".join(f"{k}={v}\n" for k, v in PROPERTIES.items()))
    for jar in addons.glob("*.jar"):
        jar.unlink()
    for p in plugins:
        shutil.copy2(p, addons / p.name)
    if world_from and not (folder / "world").exists():
        shutil.copytree(world_from, folder / "world", ignore=shutil.ignore_patterns("session.lock"))
    if jar := client_jar():
        (folder / "bluemap").mkdir(exist_ok=True)
        shutil.copy2(jar, folder / "bluemap" / jar.name)


class Server:
    """A server jar on its console (stdin/stdout); every line is echoed to `log`."""

    def __init__(self, folder: Path, jar: Path, heap: str = "2G"):
        self.folder = folder
        self.history: list[str] = []
        self.cursor = 0  # wait_for scans from here; send() moves it to the end
        self.done = False
        self.changed = threading.Condition()
        self.log = open(folder / "e2e-console.log", "a", encoding="utf-8")
        self.started = time.monotonic()
        self.proc = subprocess.Popen(
            [str(JAVA), f"-Xmx{heap}", "-jar", str(jar), "--nogui"],
            cwd=folder, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace", bufsize=1)
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self) -> None:
        for line in self.proc.stdout:
            self.log.write(line)
            self.log.flush()
            with self.changed:
                self.history.append(ANSI.sub("", line.rstrip()))
                self.changed.notify_all()
        with self.changed:
            self.done = True
            self.changed.notify_all()

    def wait_for(self, pattern: str, timeout: float = 300, since_start: bool = False) -> re.Match:
        """First line matching `pattern` after the last command (or since the server started)."""
        rx, deadline = re.compile(pattern), time.monotonic() + timeout
        i = 0 if since_start else self.cursor
        with self.changed:
            while True:
                for line in self.history[i:]:
                    i += 1
                    if m := rx.search(line):
                        return m
                if self.done:
                    raise RuntimeError(f"server exited while waiting for {pattern!r}")
                left = deadline - time.monotonic()
                if left <= 0:
                    raise TimeoutError(f"no console line matching {pattern!r} within {timeout}s")
                self.changed.wait(left)

    def send(self, command: str) -> None:
        with self.changed:
            self.cursor = len(self.history)
        self.proc.stdin.write(command + "\n")
        self.proc.stdin.flush()

    def command(self, command: str, pattern: str, timeout: float = 60) -> re.Match:
        self.send(command)
        return self.wait_for(pattern, timeout)

    def command_text(self, command: str, header: str, timeout: float = 60) -> list[str]:
        """Runs `command` and returns its multi-line message: the line matching `header` without the log prefix,
        then its continuation lines (up to the next `[hh:mm:ss …]` record)."""
        self.command(command, header, timeout)
        time.sleep(1)  # the record's continuation lines arrive just after its first line
        with self.changed:
            lines = self.history[self.cursor:]
        start = next(i for i, line in enumerate(lines) if re.search(header, line))
        block = [re.sub(r"^\[\d+:\d+:\d+ \w+\]: ", "", lines[start])]
        for line in lines[start + 1:]:
            if re.match(r"\[\d+:\d+:\d+ ", line):
                break
            block.append(line)
        return block

    def kill(self) -> None:
        """Hard-kills the JVM (SIGKILL / TerminateProcess): no shutdown hooks, the core only sees stdin EOF."""
        self.proc.kill()
        self.proc.wait(30)
        self.log.close()

    def stop(self, timeout: float = 180) -> int:
        if self.proc.poll() is None:
            self.send("stop")
        try:
            return self.proc.wait(timeout)
        except subprocess.TimeoutExpired:
            # never leave a server holding the ports for the next run
            self.proc.kill()
            self.proc.wait(30)
            return -1
        finally:
            self.log.close()


class Paper(Server):
    def __init__(self, folder: Path, heap: str = "2G"):
        super().__init__(folder, fetch("paper"), heap)


def get(path: str, timeout: float = 10) -> tuple[int, bytes]:
    try:
        with urllib.request.urlopen(f"{HTTP}/{path.lstrip('/')}", timeout=timeout) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()
    except OSError:
        return 0, b""


def poll(fn, timeout: float, interval: float = 1.0):
    """Calls `fn` until it returns something truthy; returns it (or the last falsy value on timeout)."""
    deadline, value = time.monotonic() + timeout, None
    while time.monotonic() < deadline:
        if value := fn():
            return value
        time.sleep(interval)
    return value


def pid_alive(pid: int) -> bool:
    if WINDOWS:
        out = subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/NH", "/FO", "CSV"],
                             capture_output=True, text=True).stdout
        return f'"{pid}"' in out
    try:
        # a zombie keeps its /proc entry until reaped: field 3 of stat is the state
        return Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0] != "Z"
    except (OSError, IndexError):
        return False


def rss_mib(pid: int) -> float | None:
    if WINDOWS:
        out = subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/NH", "/FO", "CSV"],
                             capture_output=True, text=True).stdout
        m = re.search(r'"([\d.,\s]+) K"', out)
        return int(re.sub(r"\D", "", m.group(1))) / 1024 if m else None
    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1]) / 1024
    return None


def kill(pid: int) -> None:
    if WINDOWS:
        subprocess.run(["taskkill", "/F", "/PID", str(pid)], capture_output=True)
    else:
        subprocess.run(["kill", "-9", str(pid)])


def json_get(path: str):
    status, body = get(path)
    return json.loads(body) if status == 200 and body else None
