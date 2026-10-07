"""Commands with a hang limit for the accept scripts.

The limit counts only time the machine was awake. Sleep / Modern Standby freezes the child and this script alike,
while the wall clock (and the child's own timers) keep running; a wall-clock limit then calls a healthy run hung the
moment the machine wakes (2026-10-07: a 4 s render "took" 1980 s across 33 min of standby and was killed). A gap
far longer than the poll interval between two polls means the machine was suspended; that gap is not counted.
"""
import subprocess
import sys
import time
from pathlib import Path

import hangdump

# seconds per CLI invocation: far above any fixture render, so only a hang reaches it
COMMAND_TIMEOUT = 600
POLL = 1.0
SUSPENDED_GAP = 10.0


def run_bounded(cmd: list[str], cwd: Path | None = None, timeout: float = COMMAND_TIMEOUT) -> subprocess.CompletedProcess:
    """`subprocess.run` that fails loudly on a hang: dumps the process (tools/hangdump.py), kills it and exits."""
    proc = subprocess.Popen(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    awake = suspended = 0.0
    last = time.monotonic()
    while True:
        try:
            out, err = proc.communicate(timeout=POLL)
            break
        except subprocess.TimeoutExpired:
            pass
        now = time.monotonic()
        step, last = now - last, now
        if step > SUSPENDED_GAP:
            suspended += step
        else:
            awake += step
        if awake > timeout and proc.poll() is None:
            evidence = hangdump.dump(proc.pid, Path(cmd[0]).stem, Path(cmd[0]))
            proc.kill()
            out, err = proc.communicate()
            sys.exit(f"HANG: {' '.join(cmd)} in {cwd} ran over {timeout:.0f}s awake\n{evidence}\n{out[-3000:]}\n{err[-3000:]}")
    if suspended:
        print(f"    (machine suspended for {suspended:.0f}s during {Path(cmd[0]).name}; not counted as running time)",
              flush=True)
    return subprocess.CompletedProcess(cmd, proc.returncode, out, err)
