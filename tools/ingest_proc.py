"""The `bluemap` process under tools/ingest_bench.py: timestamped output, /proc sampling, perf."""
import os
import re
import signal
import subprocess
import threading
import time
from pathlib import Path

MC = "26.3"


class Bluemap:
    """The process, its timestamped output and a 1 s sampler of its CPU, memory and I/O."""

    def __init__(self, exe: Path, cwd: Path, args: list[str]):
        self.lines: list[tuple[float, str]] = []
        self.samples: list[dict] = []
        self.threads: dict[str, tuple[str, float]] = {}
        self.proc = subprocess.Popen([str(exe), "-c", "config", "-v", MC, *args], cwd=cwd, stdout=subprocess.PIPE,
                                     stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace")
        self.log = open(cwd / "bluemap.log", "w", encoding="utf-8")
        self.stop_sampling = False
        threading.Thread(target=self._read, daemon=True).start()
        threading.Thread(target=self._sample, daemon=True).start()

    def _read(self) -> None:
        for line in self.proc.stdout:
            self.lines.append((time.time(), line.rstrip("\n")))
            self.log.write(line)
            self.log.flush()

    def _sample(self) -> None:
        tick = os.sysconf("SC_CLK_TCK")
        pid = self.proc.pid
        while not self.stop_sampling and self.proc.poll() is None:
            try:
                stat = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
                io = dict(l.split(": ") for l in Path(f"/proc/{pid}/io").read_text().splitlines())
                rss = int(re.search(r"VmRSS:\s+(\d+)", Path(f"/proc/{pid}/status").read_text())[1]) * 1024
                cpu_all = [int(v) for v in Path("/proc/stat").read_text().splitlines()[0].split()[1:8]]
            except (OSError, TypeError):
                break
            self.samples.append({"t": time.time(), "cpu": (int(stat[11]) + int(stat[12])) / tick, "rss": rss,
                                 "wchar": int(io["wchar"]), "rchar": int(io["rchar"]),
                                 "write_bytes": int(io["write_bytes"]), "read_bytes": int(io["read_bytes"]),
                                 "sys_busy": (sum(cpu_all) - cpu_all[3] - cpu_all[4]) / tick,
                                 "load": os.getloadavg()[0]})
            self._sample_threads()
            time.sleep(1.0)

    def wait_for(self, text: str, timeout: float) -> float:
        end = time.monotonic() + timeout
        seen = 0
        while time.monotonic() < end:
            for t, line in self.lines[seen:]:
                if text in line:
                    return t
            seen = len(self.lines)
            if self.proc.poll() is not None:
                raise SystemExit(f"bluemap exited waiting for {text!r}")
            time.sleep(0.2)
        raise SystemExit(f"timed out waiting for {text!r}")

    def thread_cpu(self) -> dict[str, float]:
        """CPU seconds by thread name, largest first; shows a serial stage that core counts hide."""
        self._sample_threads()
        out: dict[str, float] = {}
        for name, cpu in self.threads.values():
            out[name] = out.get(name, 0.0) + cpu
        return dict(sorted(((k, round(v, 1)) for k, v in out.items()), key=lambda kv: -kv[1]))

    def _sample_threads(self) -> None:
        """Per-update threads end with their task, so each thread's last seen CPU is kept (1 s resolution)."""
        tick = os.sysconf("SC_CLK_TCK")
        for task in Path(f"/proc/{self.proc.pid}/task").iterdir():
            try:
                text = (task / "stat").read_text()
            except OSError:
                continue
            fields = text.rsplit(")", 1)[1].split()
            self.threads[task.name] = (text[text.index("(") + 1:text.rindex(")")],
                                       (int(fields[11]) + int(fields[12])) / tick)

    def perf(self, seconds: int, out: Path) -> None:
        """Call-less CPU profile of the running process, by thread and symbol."""
        data = out.with_suffix(".data")
        subprocess.run(["perf", "record", "-q", "-F", "299", "-p", str(self.proc.pid), "-o", str(data), "--", "sleep",
                        str(seconds)], capture_output=True)
        report = subprocess.run(["perf", "report", "-i", str(data), "--stdio", "--sort", "comm,symbol",
                                 "--percent-limit", "0.7"], capture_output=True, text=True).stdout
        out.write_text(report)
        data.unlink(missing_ok=True)

    def terminate(self) -> tuple[float, int]:
        """Signals a clean stop; (CPU seconds, peak RSS bytes) of the whole process."""
        self.stop_sampling = True
        self.proc.send_signal(signal.SIGTERM)
        _, _, usage = os.wait4(self.proc.pid, 0)
        return usage.ru_utime + usage.ru_stime, usage.ru_maxrss * 1024
