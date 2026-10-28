"""Merge gates on Linux (the testbox): release CLI build, clippy, tests, oracles, acceptance; one line per gate.

    py -3 tools/testbox.py run <name> --sync -- python3 tools/gates.py [gate …]

Gates: fmt clippy unit mapo oracle accept accopt arm (default: all; arm = clippy for aarch64, as macOS CI runs it). Tests build in profile `fast` (cold 68 s vs 234 s for
release, 11 s vs 57 s after a bm-render edit); acceptance runs the release binary, as shipped. Stage the fixtures
first (`testbox.py stage <name>`): from the testbox HDD the oracle alone takes ~10 min.
"""
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GATES = {
    "fmt": ["cargo", "fmt", "--all", "--", "--check"],
    "clippy": ["cargo", "+stable", "clippy", "-q", "--workspace", "--all-targets", "--", "-D", "warnings"],
    "unit": ["cargo", "test", "-q", "--profile", "fast", "--workspace"],
    "mapo": ["cargo", "test", "-q", "--profile", "fast", "-p", "bm-map", "--", "--ignored"],
    "oracle": ["cargo", "test", "-q", "--profile", "fast", "-p", "bm-storage", "--test", "oracle_optimized", "--", "--ignored"],
    "accept": ["python3", "tools/accept.py", "--no-build"],
    "accopt": ["python3", "tools/accept_optimized.py", "--no-build"],
    "arm": ["cargo", "+stable", "zigbuild", "-q", "--workspace", "--all-targets", "--locked", "--target", "aarch64-unknown-linux-gnu"],
}
NEEDS_RELEASE = {"accept", "accopt"}


def arm_env() -> dict[str, str]:
    """Clippy through cargo-zigbuild (C deps need zig as cross compiler): x86 clippy never sees non-x86 cfg code."""
    home = Path.home()
    zig = home / ".local/zigenv"
    if not (zig / "bin/cargo-zigbuild").exists():
        subprocess.run(["uv", "venv", "-q", str(zig)], check=True)
        subprocess.run(["uv", "pip", "install", "-q", "--python", str(zig), "ziglang", "cargo-zigbuild"], check=True)
    subprocess.run(["rustup", "-q", "target", "add", "--toolchain", "stable", "aarch64-unknown-linux-gnu"], check=True)
    return {
        **os.environ,
        "PATH": f"{zig / 'bin'}:{os.environ['PATH']}",
        "RUSTC_WORKSPACE_WRAPPER": str(home / ".rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/clippy-driver"),
        "CLIPPY_ARGS": "-D__CLIPPY_HACKERY__warnings__CLIPPY_HACKERY__",
        "RUSTFLAGS": "-D warnings",
    }


def run(name: str, cmd: list[str]) -> bool:
    started = time.monotonic()
    log = Path(f"/tmp/gates-{ROOT.name}-{name}.log")
    env = arm_env() if name == "arm" else None
    with log.open("wb") as out:
        ok = subprocess.run(cmd, cwd=ROOT, env=env, stdout=out, stderr=subprocess.STDOUT).returncode == 0
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
