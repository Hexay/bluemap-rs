"""Builds the core binary per plugin target and stages it in platforms/natives/<target>/ (docs/13 §5).
Shared by CI (.github/workflows/*.yml) and local builds.

    py -3 tools/build_core.py [target…] [--jobs N] [--jars] [--list]

Targets: windows-x64 (native MSVC), linux-x64 / linux-arm64 / linux-armv7 (static musl via cargo-zigbuild, from any
host), macos-x64 / macos-arm64 (native cargo, macOS hosts only). No target = the host's own, unless --jars is given alone.
--jars then runs Gradle `allJars`: per shim (Paper, Fabric) one jar per staged target plus the universal jar
(platforms/<shim>/build/libs); Gradle needs Java 25 (JAVA_HOME, or work/downloads/jdk25 when present).
cargo-zigbuild and zig come from PATH or `pip install cargo-zigbuild ziglang` (pinned in CI).
"""
import argparse
import os
import platform
import shutil
import subprocess
import sys
import sysconfig
from pathlib import Path

from paths import ROOT, WINDOWS, jdk_dir

PLATFORMS = ROOT / "platforms"
NATIVES = PLATFORMS / "natives"
SHIMS = ("paper", "fabric")

# plugin target -> (rust triple, builder)
TARGETS = {
    "windows-x64": ("x86_64-pc-windows-msvc", "cargo"),
    "linux-x64": ("x86_64-unknown-linux-musl", "zigbuild"),
    "linux-arm64": ("aarch64-unknown-linux-musl", "zigbuild"),
    "linux-armv7": ("armv7-unknown-linux-musleabihf", "zigbuild"),
    "macos-x64": ("x86_64-apple-darwin", "cargo"),
    "macos-arm64": ("aarch64-apple-darwin", "cargo"),
}

# per-triple C flags for cc-rs builds (zstd, libdeflate, zlib, ring)
CFLAGS = {
    # zig's clang rejects libdeflate's AVX-512 crc32 kernel without the evex512 feature
    "x86_64-unknown-linux-musl": "-mevex512",
}


def host_target() -> str:
    machine = platform.machine().lower()
    arch = {"amd64": "x64", "x86_64": "x64", "arm64": "arm64", "aarch64": "arm64"}.get(machine, machine)
    os_name = "windows" if WINDOWS else "macos" if sys.platform == "darwin" else "linux"
    return f"{os_name}-{arch}"


def binary_name(target: str) -> str:
    return "bluemap-core.exe" if target.startswith("windows") else "bluemap-core"


def zig_env() -> dict[str, str]:
    """PATH with zig and cargo-zigbuild, falling back to the pip packages' install dirs."""
    env = dict(os.environ)
    extra = []
    if not shutil.which("zig"):
        try:
            import ziglang
            extra.append(str(Path(ziglang.__file__).parent))
        except ImportError:
            sys.exit("zig not found: pip install ziglang (or put zig on PATH)")
    if not shutil.which("cargo-zigbuild"):
        extra += [sysconfig.get_path("scripts"), sysconfig.get_path("scripts", f"{os.name}_user")]
    env["PATH"] = os.pathsep.join([*extra, env.get("PATH", "")])
    return env


def build(target: str, jobs: int | None) -> Path:
    triple, builder = TARGETS[target]
    env = zig_env() if builder == "zigbuild" else dict(os.environ)
    # line tables stay in the MSVC .pdb but would quadruple an ELF (67 vs ~16 MB); symbols keep backtraces named
    env["CARGO_PROFILE_RELEASE_STRIP"] = "debuginfo"
    if flags := CFLAGS.get(triple):
        env[f"CFLAGS_{triple.replace('-', '_')}"] = flags
    cmd = ["cargo", "zigbuild"] if builder == "zigbuild" else ["cargo", "build"]
    cmd += ["--release", "--locked", "-p", "bm-cli", "--target", triple]
    if jobs:
        cmd += ["-j", str(jobs)]
    print(f"== {target}: {' '.join(cmd)}", flush=True)
    subprocess.run(cmd, cwd=ROOT, env=env, check=True)
    built = ROOT / "target" / triple / "release" / ("bluemap.exe" if target.startswith("windows") else "bluemap")
    dest = NATIVES / target / binary_name(target)
    dest.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(built, dest)
    return dest


def build_jars(shims: tuple[str, ...] = SHIMS) -> list[Path]:
    gradlew = PLATFORMS / ("gradlew.bat" if WINDOWS else "gradlew")
    cmd = [str(gradlew), *(f":{s}:allJars" for s in shims), "--no-daemon", "--console=plain", "-q", "--max-workers=2"]
    env = dict(os.environ)
    if jdk_dir(25).is_dir():
        env["JAVA_HOME"] = str(jdk_dir(25))
    subprocess.run(cmd, cwd=PLATFORMS, check=True, env=env)
    return sorted(j for s in shims for j in (PLATFORMS / s / "build" / "libs").glob(f"bluemap-rs-{s}-*.jar"))


def stage_host_core(core: Path | None, jobs: int | None) -> None:
    """Stages the host target's core for the e2e tests: a prebuilt `core`, else one built here."""
    target = host_target()
    if not core:
        build(target, jobs)
        return
    native = NATIVES / target / binary_name(target)
    native.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(core, native)
    native.chmod(0o755)


def host_jar(shim: str) -> Path:
    """The host target's jar of `shim` from the last `build_jars`."""
    libs = PLATFORMS / shim / "build" / "libs"
    jars = sorted(libs.glob(f"*-{host_target()}.jar"))
    if not jars:
        sys.exit(f"no {host_target()} jar in {libs}; run without --skip-build")
    return jars[-1]


def mib(path: Path) -> str:
    return f"{path.stat().st_size / 2**20:6.1f} MiB  {path.relative_to(ROOT)}"


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("targets", nargs="*", metavar="target", help=", ".join(TARGETS))
    ap.add_argument("--jobs", "-j", type=int)
    ap.add_argument("--jars", action="store_true", help="build the plugin jars from everything staged in natives/")
    ap.add_argument("--list", action="store_true", help="print the targets and exit")
    args = ap.parse_args()
    if args.list:
        print("\n".join(f"{t} {triple}" for t, (triple, _) in TARGETS.items()))
        return
    targets = args.targets or ([] if args.jars else [host_target()])
    if unknown := [t for t in targets if t not in TARGETS]:
        ap.error(f"unknown target(s) {unknown}; known: {', '.join(TARGETS)}")
    staged = [build(t, args.jobs) for t in targets]
    jars = build_jars() if args.jars else []
    for path in [*staged, *jars]:
        print(mib(path))


if __name__ == "__main__":
    main()
