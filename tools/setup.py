"""Download a toolchain's JDK, Minecraft server jar and BlueMap CLI into work/downloads and generate the
vanilla data reports. Idempotent. Without arguments: the default (26.3) toolchain.

Usage: py -3 tools/setup.py [--mc 1.21.11 [--bluemap 5.28]]
"""
import argparse
import hashlib
import json
import shutil
import struct
import subprocess
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

from console import bundler_args
from paths import DEFAULT, DOWNLOADS, EXE, MC_MANIFEST_URL, WINDOWS, Toolchain, jdk_dir, jdk_url, toolchain


def open_url(url: str):
    # Adoptium/GitHub answer 403 to urllib's default User-Agent
    return urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "bluemap-rs-setup"}))


def fetch_json(url: str) -> dict:
    with open_url(url) as r:
        return json.load(r)


def download(url: str, dest: Path, sha1: str | None = None) -> None:
    if dest.exists() and (sha1 is None or file_sha1(dest) == sha1):
        print(f"ok      {dest.name}")
        return
    print(f"fetch   {dest.name} <- {url}")
    tmp = dest.with_suffix(dest.suffix + ".part")
    with open_url(url) as r, open(tmp, "wb") as f:
        shutil.copyfileobj(r, f)
    if sha1 is not None and file_sha1(tmp) != sha1:
        tmp.unlink()
        sys.exit(f"sha1 mismatch for {dest.name}")
    tmp.replace(dest)


def file_sha1(path: Path) -> str:
    h = hashlib.sha1()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def version_json(mc: str) -> dict:
    manifest = fetch_json(MC_MANIFEST_URL)
    entry = next((v for v in manifest["versions"] if v["id"] == mc), None)
    if entry is None:
        sys.exit(f"unknown Minecraft version {mc}")
    return fetch_json(entry["url"])


def resolve(mc: str, bluemap: str) -> Toolchain:
    """Toolchain for `mc` + `bluemap`: the server's Java from Mojang's metadata, BlueMap's from its jar."""
    if mc == DEFAULT.mc and bluemap == DEFAULT.bluemap:
        return DEFAULT
    DOWNLOADS.mkdir(parents=True, exist_ok=True)
    probe = toolchain(mc, bluemap, 0, 0)
    download(probe.bluemap_url, probe.bluemap_jar)
    return toolchain(mc, bluemap, version_json(mc)["javaVersion"]["majorVersion"], jar_java_major(probe.bluemap_jar))


def jar_java_major(jar: Path, main_class: str = "de/bluecolored/bluemap/cli/BlueMapCLI.class") -> int:
    """Java release a jar was compiled for: class-file major version − 44."""
    with zipfile.ZipFile(jar) as z:
        header = z.read(main_class)[:8]
    return struct.unpack(">H", header[6:8])[0] - 44


def install_jdk(major: int) -> None:
    target = jdk_dir(major)
    if (target / "bin" / f"java{EXE}").exists():
        print(f"ok      {target.name}")
        return
    archive = DOWNLOADS / f"jdk{major}.{'zip' if WINDOWS else 'tar.gz'}"
    download(jdk_url(major), archive)
    staging = DOWNLOADS / "jdk-staging"
    shutil.rmtree(staging, ignore_errors=True)
    if WINDOWS:
        with zipfile.ZipFile(archive) as z:
            z.extractall(staging)
    else:
        with tarfile.open(archive) as t:
            t.extractall(staging, filter="tar")
    # the archive holds a single versioned top-level folder, e.g. jdk-25.0.4.1+1/
    (top,) = staging.iterdir()
    top.rename(target)
    staging.rmdir()
    archive.unlink()


def download_server_jar(tc: Toolchain) -> None:
    server = version_json(tc.mc)["downloads"]["server"]
    download(server["url"], tc.server_jar, server["sha1"])


def generate_reports(tc: Toolchain) -> None:
    if tc.blocks_json.exists():
        print(f"ok      {tc.blocks_json.relative_to(tc.reports.parent)}")
        return
    print("gen     vanilla data reports")
    tc.reports.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        [str(tc.java), "-DbundlerMainClass=net.minecraft.data.Main", *bundler_args(tc), "-jar", str(tc.server_jar),
         "--reports", "--output", str(tc.reports)],
        cwd=tc.reports.parent, check=True, stdout=subprocess.DEVNULL,
    )


def setup(tc: Toolchain) -> None:
    DOWNLOADS.mkdir(parents=True, exist_ok=True)
    install_jdk(tc.java_major)
    install_jdk(tc.bluemap_java_major)
    download_server_jar(tc)
    download(tc.bluemap_url, tc.bluemap_jar)
    generate_reports(tc)
    print(f"java    {tc.java}")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mc", default=DEFAULT.mc)
    ap.add_argument("--bluemap", default=DEFAULT.bluemap)
    args = ap.parse_args()
    setup(resolve(args.mc, args.bluemap))


if __name__ == "__main__":
    main()
