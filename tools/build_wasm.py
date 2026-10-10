"""Rebuild the checked-in WASM unpacker that bm-web ships inside its client script (docs/18).

    py -3 tools/build_wasm.py

Needs `rustup target add wasm32-unknown-unknown`. Run it after changing bm_format::compact's decoder or bm-wasm.
"""
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = "wasm32-unknown-unknown"
BUILT = ROOT / "target" / TARGET / "release" / "bm_wasm.wasm"
SHIPPED = ROOT / "crates" / "bm-web" / "client" / "unpack.wasm"


def main() -> int:
    # no debug info or symbols: the module is base64'd into a script every viewer downloads
    cmd = ["cargo", "build", "--release", "-p", "bm-wasm", "--lib", "--target", TARGET,
           "--config", "profile.release.debug=false", "--config", "profile.release.strip=true",
           "--config", "profile.release.panic='abort'"]
    done = subprocess.run(cmd, cwd=ROOT)
    if done.returncode != 0:
        return done.returncode
    before = SHIPPED.read_bytes() if SHIPPED.exists() else b""
    SHIPPED.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(BUILT, SHIPPED)
    state = "unchanged" if before == SHIPPED.read_bytes() else "updated"
    print(f"{SHIPPED.relative_to(ROOT)}: {SHIPPED.stat().st_size} bytes ({state})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
