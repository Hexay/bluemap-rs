"""Headless Paper server for the plugin e2e test: pinned downloads, server folder setup, console + HTTP helpers."""
import hashlib
import json
import queue
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
}

PROPERTIES = {
    "online-mode": "false", "server-port": "25590", "allow-flight": "true", "view-distance": "4",
    "simulation-distance": "4", "level-seed": "bluemap-e2e", "spawn-protection": "0", "motd": "bluemap e2e",
}


def fetch(key: str) -> Path:
    name, url, sha = DOWNLOADS[key]
    path = CACHE / name
    if not path.is_file():
        CACHE.mkdir(parents=True, exist_ok=True)
        print(f"downloading {name}", flush=True)
        tmp = path.with_suffix(".part")
        with urllib.request.urlopen(url, timeout=120) as r, open(tmp, "wb") as f:
            shutil.copyfileobj(r, f)
        tmp.replace(path)
    if sha and hashlib.sha256(path.read_bytes()).hexdigest() != sha:
        raise RuntimeError(f"checksum mismatch: {path}")
    return path


def client_jar() -> Path | None:
    """A cached 26.3 client jar, so neither BlueMap has to download it."""
    for p in (WORK / "bluemap").glob(f"*/data/minecraft-client-{MC}.jar"):
        return p
    return None


def prepare(folder: Path, plugins: list[Path], fresh: bool, world_from: Path | None = None) -> None:
    if fresh and folder.exists():
        shutil.rmtree(folder)
    (folder / "plugins").mkdir(parents=True, exist_ok=True)
    (folder / "eula.txt").write_text("eula=true\n")
    (folder / "server.properties").write_text("".join(f"{k}={v}\n" for k, v in PROPERTIES.items()))
    for jar in (folder / "plugins").glob("*.jar"):
        jar.unlink()
    for p in plugins:
        shutil.copy2(p, folder / "plugins" / p.name)
    if world_from and not (folder / "world").exists():
        shutil.copytree(world_from, folder / "world", ignore=shutil.ignore_patterns("session.lock"))
    if jar := client_jar():
        (folder / "bluemap").mkdir(exist_ok=True)
        shutil.copy2(jar, folder / "bluemap" / jar.name)


class Paper:
    """A Paper server on its console (stdin/stdout); every line is echoed to `log`."""

    def __init__(self, folder: Path, heap: str = "2G"):
        self.folder = folder
        self.lines: queue.Queue[str | None] = queue.Queue()
        self.log = open(folder / "e2e-console.log", "a", encoding="utf-8")
        self.started = time.monotonic()
        self.proc = subprocess.Popen(
            [str(JAVA), f"-Xmx{heap}", "-jar", str(fetch("paper")), "--nogui"],
            cwd=folder, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace", bufsize=1)
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self) -> None:
        for line in self.proc.stdout:
            self.log.write(line)
            self.log.flush()
            self.lines.put(line.rstrip())
        self.lines.put(None)

    def wait_for(self, pattern: str, timeout: float = 300) -> re.Match:
        rx, deadline = re.compile(pattern), time.monotonic() + timeout
        while (left := deadline - time.monotonic()) > 0:
            try:
                line = self.lines.get(timeout=left)
            except queue.Empty:
                break
            if line is None:
                raise RuntimeError(f"server exited while waiting for {pattern!r}")
            if m := rx.search(line):
                return m
        raise TimeoutError(f"no console line matching {pattern!r} within {timeout}s")

    def send(self, command: str) -> None:
        self.proc.stdin.write(command + "\n")
        self.proc.stdin.flush()

    def command(self, command: str, pattern: str, timeout: float = 60) -> re.Match:
        self.send(command)
        return self.wait_for(pattern, timeout)

    def stop(self, timeout: float = 180) -> int:
        if self.proc.poll() is None:
            self.send("stop")
        try:
            return self.proc.wait(timeout)
        finally:
            self.log.close()


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
    return Path(f"/proc/{pid}").exists()


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
