"""Parity on older Minecraft versions: per version, its vanilla server generates the fixture world, Java BlueMap 5.28
renders it (`-v <mc>`: that version's client jar, plus 1.19.4's for data packs before 1.19.4), our `bluemap` renders a
copy of the same config; the webroots must be identical and `-r` over Java's webroot must render nothing.

Usage: py -3 tools/accept_versions.py [mc ...] [--fixture vanilla] [--rerender] [--no-build]
Default versions: 1.20.1 1.21.1 1.21.4. Output: work/v/<mc>/{worlds,bluemap}/<fixture>, work/accept/<fixture>-<mc>[-dropin].
Exit 1 if any check fails.
"""
import argparse
import subprocess
import sys
from pathlib import Path

from accept import ROOT, bluemap, check, compare, prepare
from make_world import make_world
from setup import resolve, setup

TOOLS = Path(__file__).resolve().parent
VERSIONS = ["1.20.1", "1.21.1", "1.21.4"]
BLUEMAP = "5.28"


def accept_version(mc: str, fixture: str, rerender: bool, failures: list[str]) -> None:
    print(f"== {mc}", flush=True)
    tc = resolve(mc, BLUEMAP)
    setup(tc)
    # its own port: tools/real_world.py's server may be generating at the same time
    make_world(fixture, False, tc, port=25611)
    golden = tc.bluemap_root / fixture / "web"
    if rerender or not (golden / "maps").is_dir():
        code = subprocess.call([sys.executable, str(TOOLS / "render_serve.py"), fixture, "--no-serve", "--force-render",
                                "--mc", mc, "--bluemap", BLUEMAP], stdout=subprocess.DEVNULL)
        if code:
            failures.append(f"{mc}: Java render failed ({code})")
            return
    out = prepare(fixture, f"{fixture}-{mc}", tc=tc)
    bluemap(out, "-r", mc=mc)
    check(compare(golden, out / "web"), f"{mc}: webroot equals Java's", failures)
    dropin = prepare(fixture, f"{fixture}-{mc}-dropin", golden, tc=tc)
    maps = bluemap(dropin, "-r", mc=mc)
    check(all(r == 0 for r, _ in maps.values()), f"{mc}: -r over Java's webroot renders nothing (drop-in)", failures)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("versions", nargs="*", default=VERSIONS)
    ap.add_argument("--fixture", default="vanilla")
    ap.add_argument("--rerender", action="store_true", help="re-run Java even if its webroot exists")
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release", "-p", "bm-cli", "-p", "bm-golden"], cwd=ROOT, check=True)
    failures: list[str] = []
    for mc in args.versions:
        accept_version(mc, args.fixture, args.rerender, failures)
    print(f"\n{len(failures)} failed" + "".join(f"\n  {f}" for f in failures))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
