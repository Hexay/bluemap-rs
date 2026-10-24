"""Merge gates on Linux (the testbox): release CLI build, clippy, tests, oracles, acceptance; one line per gate.

    py -3 tools/testbox.py run <name> --sync -- python3 tools/gates.py [gate …]

Gates: clippy unit mapo oracle accept accopt (default: all). Tests build in profile `fast` (cold 68 s vs 234 s for
release, 11 s vs 57 s after a bm-render edit); acceptance runs the release binary, as shipped. Stage the fixtures
first (`testbox.py stage <name>`): from the testbox HDD the oracle alone takes ~10 min.
"""
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GATES = {
    "clippy": ["cargo", "+stable", "clippy", "-q", "--workspace", "--all-targets", "--", "-D", "warnings"],
    "unit": ["cargo", "test", "-q", "--profile", "fast", "--workspace"],
    "mapo": ["cargo", "test", "-q", "--profile", "fast", "-p", "bm-map", "--", "--ignored"],
    "oracle": ["cargo", "test", "-q", "--profile", "fast", "-p", "bm-storage", "--test", "oracle_optimized", "--", "--ignored"],
    "accept": ["python3", "tools/accept.py", "--no-build"],
    "accopt": ["python3", "tools/accept_optimized.py", "--no-build"],
}
NEEDS_RELEASE = {"accept", "accopt"}


def run(name: str, cmd: list[str]) -> bool:
    started = time.monotonic()
    log = Path(f"/tmp/gates-{ROOT.name}-{name}.log")
    with log.open("wb") as out:
        ok = subprocess.run(cmd, cwd=ROOT, stdout=out, stderr=subprocess.STDOUT).returncode == 0
    print(f"{'PASS' if ok else 'FAIL'} {name} {time.monotonic() - started:.0f}s", flush=True)
    if not ok:
        print("".join(log.read_text(errors="replace").splitlines(keepends=True)[-15:]), flush=True)
    return ok


def main() -> None:
    picked = sys.argv[1:] or list(GATES)
    unknown = [g for g in picked if g not in GATES]
    if unknown:
        sys.exit(f"unknown gates {unknown}; pick from {list(GATES)}")
    if not str((ROOT / "work/bluemap").resolve()).startswith("/dev/shm/"):
        print("warning: work/bluemap is not on tmpfs; fixture gates will be disk-bound (testbox.py stage)", flush=True)
    built = not NEEDS_RELEASE & set(picked) or run("build", ["cargo", "build", "-q", "--release", "-p", "bm-cli", "-p", "bm-golden"])
    ok = built
    for name in picked:
        if built or name not in NEEDS_RELEASE:
            ok = run(name, GATES[name]) and ok
    print("ALL PASS" if ok else "GATES FAILED")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
