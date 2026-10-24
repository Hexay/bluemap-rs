"""Watch-mode soak (Linux, testbox): `bluemap -u` under tools/rss_sample.py while a fake explorer edits the world.

    python3 tools/soak_watch.py --bin BIN --cwd FIXTURE --region DIR --staging DIR [--minutes M] [--seed N]
                                [--add-every S] [--touch K] [--csv FILE] [--fresh] [-- extra bluemap args…]

Empties `--region`, seeds it with the N staged region files nearest the origin, starts `bluemap -c config -u`, then
every `--add-every` seconds moves the next-nearest staged region in (the frontier a bot explores) and rewrites K random
present regions under a new mtime (copy + rename, same bytes: chunks the bot revisits). After M minutes it sends
SIGINT and prints the sampler's RSS summary; `--csv` keeps the timeline.
"""
import argparse
import os
import random
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

REGION = re.compile(r"r\.(-?\d+)\.(-?\d+)\.mca$")


def by_distance(files: list[Path]) -> list[Path]:
    def key(p: Path):
        x, z = map(int, REGION.search(p.name).groups())
        return (x + 0.5) ** 2 + (z + 0.5) ** 2, x, z
    return sorted((p for p in files if REGION.search(p.name)), key=key)


def rewrite(path: Path) -> None:
    tmp = path.with_suffix(".tmp")
    shutil.copyfile(path, tmp)
    os.replace(tmp, path)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", required=True)
    ap.add_argument("--cwd", required=True)
    ap.add_argument("--region", required=True, type=Path)
    ap.add_argument("--staging", required=True, type=Path)
    ap.add_argument("--minutes", type=float, default=25)
    ap.add_argument("--seed", type=int, default=16)
    ap.add_argument("--add-every", type=float, default=20)
    ap.add_argument("--touch", type=int, default=2)
    ap.add_argument("--csv")
    ap.add_argument("--fresh", action="store_true", help="delete <cwd>/web/maps first (else seeded tiles are skipped)")
    ap.add_argument("extra", nargs=argparse.REMAINDER)
    a = ap.parse_args()
    extra = a.extra[1:] if a.extra[:1] == ["--"] else a.extra
    if a.fresh:
        shutil.rmtree(Path(a.cwd) / "web" / "maps", ignore_errors=True)
    a.region.mkdir(parents=True, exist_ok=True)
    for f in a.region.iterdir():
        f.unlink()
    staged = by_distance(list(a.staging.iterdir()))
    for f in staged[: a.seed]:
        os.link(f, a.region / f.name)
    pending = staged[a.seed :]
    sampler = [sys.executable, str(Path(__file__).with_name("rss_sample.py")), "--interval", "1", "--buckets", "25"]
    if a.csv:
        sampler += ["--csv", a.csv]
    proc = subprocess.Popen([*sampler, "--", a.bin, "-c", "config", "-u", *extra], cwd=a.cwd)
    rng = random.Random(1)
    end = time.monotonic() + a.minutes * 60
    added = touched = 0
    while time.monotonic() < end and proc.poll() is None:
        time.sleep(min(a.add_every, max(0.0, end - time.monotonic())))
        if pending:
            f = pending.pop(0)
            os.link(f, a.region / f.name)
            added += 1
        present = sorted(a.region.glob("r.*.mca"))
        for f in rng.sample(present, min(a.touch, len(present))):
            rewrite(f)
            touched += 1
    proc.send_signal(2)
    code = proc.wait()
    print(f"== soak: {a.minutes:g} min, {added} regions added, {touched} rewrites, exit {code}", file=sys.stderr)
    return code


if __name__ == "__main__":
    sys.exit(main())
