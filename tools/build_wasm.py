"""Rebuild the checked-in WASM unpacker that bm-web ships inside its client script (docs/18).

    py -3 tools/build_wasm.py

Needs `rustup target add wasm32-unknown-unknown` and clang + llvm-ar with the wasm32 target (zstd's C decoder;
override with CC_/AR_wasm32_unknown_unknown). Run it after changing bm_format::compact's decoder or bm-wasm.
"""
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = "wasm32-unknown-unknown"
ENV_TARGET = TARGET.replace("-", "_")
TARGET_DIR = ROOT / "target" / "wasm"
ARCHIVE = TARGET_DIR / TARGET / "release" / "libbm_wasm.a"
SHIPPED = ROOT / "crates" / "bm-web" / "client" / "unpack.wasm"
EXPORTS = ["bmq3_input", "bmq3_output", "bmq3_unpack", "bmq3_unpack_frame"]
# decoder-only size options (same output); see docs/18 "Clients without zstd" for the measured variants
ZSTD_CFLAGS = ("-Os -DHUF_FORCE_DECOMPRESS_X1 -DZSTD_FORCE_DECOMPRESS_SEQUENCES_SHORT -DZSTD_STRIP_ERROR_STRINGS "
               "-DZSTD_NO_TRACE")


def rust_lld() -> Path:
    sysroot = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True, check=True).stdout.strip()
    host = next(line.split(": ")[1] for line in subprocess.run(["rustc", "-vV"], capture_output=True, text=True,
                                                               check=True).stdout.splitlines() if line.startswith("host: "))
    return Path(sysroot) / "lib" / "rustlib" / host / "bin" / ("rust-lld.exe" if os.name == "nt" else "rust-lld")


def main() -> int:
    env = dict(os.environ)
    env.setdefault(f"CC_{ENV_TARGET}", "clang")
    env.setdefault(f"AR_{ENV_TARGET}", "llvm-ar")
    env[f"CFLAGS_{ENV_TARGET}"] = ZSTD_CFLAGS
    if not shutil.which(env[f"CC_{ENV_TARGET}"]):
        print(f"no {env[f'CC_{ENV_TARGET}']}: zstd's decoder needs clang with the wasm32 target", file=sys.stderr)
        return 1
    # a staticlib linked here, not a cdylib: a cdylib exports (and keeps) every upstream #[no_mangle] function,
    # zstd-sys's wasm shim included (~35 KB)
    cmd = ["cargo", "rustc", "--release", "-p", "bm-wasm", "--lib", "--crate-type", "staticlib", "--target", TARGET,
           "--target-dir", str(TARGET_DIR), "--config", "profile.release.debug=false",
           "--config", "profile.release.panic='abort'"]
    if subprocess.run(cmd, cwd=ROOT, env=env).returncode != 0:
        return 1
    built = TARGET_DIR / "unpack.wasm"
    link = [str(rust_lld()), "-flavor", "wasm", "--no-entry", "--gc-sections", "--strip-all", "-O2",
            "-z", "stack-size=1048576", "--stack-first", *(f"--export={name}" for name in EXPORTS),
            "--whole-archive", str(ARCHIVE), "-o", str(built)]
    if subprocess.run(link, cwd=ROOT).returncode != 0:
        return 1
    before = SHIPPED.read_bytes() if SHIPPED.exists() else b""
    SHIPPED.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(built, SHIPPED)
    state = "unchanged" if before == SHIPPED.read_bytes() else "updated"
    print(f"{SHIPPED.relative_to(ROOT)}: {SHIPPED.stat().st_size} bytes ({state})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
