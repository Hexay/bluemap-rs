"""Drive a headless Minecraft server over stdin/stdout."""
import queue
import re
import subprocess
import threading
import time
from pathlib import Path

from paths import DEFAULT, WORK, Toolchain

COMMAND_ERRORS = re.compile(
    r"/ERROR\]|Too many blocks|No blocks were filled|Unknown or incomplete command|Incorrect argument|not loaded|Invalid|Expected|Could not|Unknown block"
)


def bundler_args(tc: Toolchain) -> list[str]:
    """The server jar unpacks its libraries into the working directory unless given a repo. One repo for
    all versions: the first JVM to load fresh jars pays ~2 min (virus scan), later runs and versions reuse them."""
    return [f"-DbundlerRepoDir={WORK / 'server-libs'}"]


class ServerTimeout(RuntimeError):
    pass


class Server:
    def __init__(self, cwd: Path, heap: str = "4G", tc: Toolchain = DEFAULT):
        self.lines: queue.Queue[str] = queue.Queue()
        self.errors: list[str] = []
        self.label = cwd.name
        self.proc = subprocess.Popen(
            [str(tc.java), f"-Xmx{heap}", *bundler_args(tc), "-jar", str(tc.server_jar), "--nogui"],
            cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="replace", bufsize=1,
        )
        threading.Thread(target=self._pump, daemon=True).start()

    def _pump(self) -> None:
        for line in self.proc.stdout:
            line = line.rstrip()
            if COMMAND_ERRORS.search(line):
                self.errors.append(line)
            self.lines.put(line)
        self.lines.put(None)

    def wait_for(self, pattern: str, timeout: float = 300, echo: bool = False) -> re.Match:
        rx = re.compile(pattern)
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise ServerTimeout(f"no line matching {pattern!r} within {timeout}s")
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty:
                continue
            if line is None:
                raise RuntimeError(f"server exited while waiting for {pattern!r}")
            if echo:
                print(f"  {self.label} | {line}", flush=True)
            if m := rx.search(line):
                return m

    def send(self, command: str) -> None:
        self.proc.stdin.write(command.lstrip("/") + "\n")
        self.proc.stdin.flush()

    def query(self, command: str, pattern: str, timeout: float = 30) -> re.Match:
        self.send(command)
        return self.wait_for(pattern, timeout)

    def stop(self, timeout: float = 120) -> int:
        self.send("stop")
        return self.proc.wait(timeout)
