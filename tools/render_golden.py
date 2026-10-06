"""Render every fixture that has a world with Java BlueMap: the golden outputs bluemap-rs is diffed against.

Usage: py -3 tools/render_golden.py [fixture ...] [--mc 26.3] [--bluemap 5.28]
Output: <toolchain>/bluemap/<fixture>/web/maps/<id>/ (see render_serve.py for the layout).
"""
import argparse
import subprocess
import sys
import time
from pathlib import Path

from paths import DEFAULT

TOOLS = Path(__file__).resolve().parent


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("fixtures", nargs="*")
    ap.add_argument("--mc", default=DEFAULT.mc)
    ap.add_argument("--bluemap", default=DEFAULT.bluemap)
    args = ap.parse_args()
    from setup import resolve, setup

    tc = resolve(args.mc, args.bluemap)
    setup(tc)
    fixtures = args.fixtures or sorted(p.name for p in tc.worlds.iterdir() if (p / "world").is_dir())
    failed = []
    for fixture in fixtures:
        start = time.monotonic()
        code = subprocess.call(
            [sys.executable, str(TOOLS / "render_serve.py"), fixture, "--no-serve", "--force-render",
             "--mc", args.mc, "--bluemap", args.bluemap],
            stdout=subprocess.DEVNULL,
        )
        print(f"{'ok' if code == 0 else 'FAIL':7} {fixture} ({time.monotonic() - start:.0f}s)", flush=True)
        if code:
            failed.append(fixture)
    if failed:
        sys.exit(f"failed: {' '.join(failed)}")


if __name__ == "__main__":
    main()
