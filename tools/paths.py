"""Pinned versions and on-disk layout shared by all tools. Everything under work/ is git-ignored.

`Toolchain` = one Minecraft + BlueMap (+ Java) combination. The default (26.3) keeps the original
work/{worlds,bluemap,cache} layout; other versions live under work/v/<mc>/. Downloads are shared.
"""
import os
from dataclasses import dataclass
from pathlib import Path

# Windows (dev) or Linux (the testbox for RAM-heavy fixtures)
WINDOWS = os.name == "nt"
EXE = ".exe" if WINDOWS else ""

ROOT = Path(__file__).resolve().parent.parent
WORK = ROOT / "work"
# a junction to bluemap_reverse/work/downloads on the dev box: JDKs and jars are shared, not re-fetched
DOWNLOADS = WORK / "downloads"
FIXTURES = ROOT / "fixtures"
RESULTS = ROOT / "docs" / "results"

MC_MANIFEST_URL = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"

WEB_HOST = "127.0.0.1"
WEB_PORT = 8100


def jdk_dir(major: int) -> Path:
    return DOWNLOADS / f"jdk{major}"


def jdk_url(major: int) -> str:
    return f"https://api.adoptium.net/v3/binary/latest/{major}/ga/{'windows' if WINDOWS else 'linux'}/x64/jdk/hotspot/normal/eclipse"


@dataclass(frozen=True)
class Toolchain:
    mc: str
    bluemap: str
    # the server's Java (Mojang metadata) and BlueMap's (its jar's class-file version) differ:
    # e.g. Minecraft 1.21 runs on Java 21 but BlueMap 5.27 needs 25
    java_major: int
    bluemap_java_major: int
    # root for per-version worlds / BlueMap configs / mirrors
    work: Path

    @property
    def java(self) -> Path:
        return jdk_dir(self.java_major) / "bin" / f"java{EXE}"

    @property
    def bluemap_java(self) -> Path:
        return jdk_dir(self.bluemap_java_major) / "bin" / f"java{EXE}"

    @property
    def server_jar(self) -> Path:
        return DOWNLOADS / f"minecraft-server-{self.mc}.jar"

    @property
    def bluemap_jar(self) -> Path:
        return DOWNLOADS / f"bluemap-{self.bluemap}-cli.jar"

    @property
    def bluemap_url(self) -> str:
        return f"https://github.com/BlueMap-Minecraft/BlueMap/releases/download/v{self.bluemap}/bluemap-{self.bluemap}-cli.jar"

    @property
    def reports(self) -> Path:
        """Vanilla data reports; blocks.json at reports/reports/blocks.json."""
        return WORK / "data" / f"reports-{self.mc}"

    @property
    def blocks_json(self) -> Path:
        return self.reports / "reports" / "blocks.json"

    @property
    def worlds(self) -> Path:
        return self.work / "worlds"

    @property
    def bluemap_root(self) -> Path:
        return self.work / "bluemap"

DEFAULT = Toolchain(mc="26.3", bluemap="5.28", java_major=25, bluemap_java_major=25, work=WORK)


def toolchain(mc: str, bluemap: str, java_major: int, bluemap_java_major: int) -> Toolchain:
    work = WORK if mc == DEFAULT.mc else WORK / "v" / mc
    return Toolchain(mc=mc, bluemap=bluemap, java_major=java_major, bluemap_java_major=bluemap_java_major, work=work)


# default-toolchain aliases used by the original single-version tools
WORLDS = DEFAULT.worlds
BLUEMAP = DEFAULT.bluemap_root
