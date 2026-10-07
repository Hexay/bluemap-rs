"""Disk inventory of BlueMap webroots: bytes by category, logical vs NTFS-allocated, Java vs Rust per-file diff.

Usage: py -3 -I docs/perf-exp/disk_inventory.py <webroot> [<webroot> ...]
  Pairs given as A=B compare hires/lowres per file (sizes, byte-identical, decompressed-identical).
Allocated size comes from GetFileInformationByHandleEx(FileStandardInfo), so MFT-resident files count 0.
"""
import ctypes
import ctypes.wintypes as wt
import gzip
import io
import sys
from collections import defaultdict
from pathlib import Path

k32 = ctypes.WinDLL("kernel32", use_last_error=True)
k32.CreateFileW.restype = wt.HANDLE


class FileStandardInfo(ctypes.Structure):
    _fields_ = [("alloc", ctypes.c_longlong), ("eof", ctypes.c_longlong), ("links", wt.DWORD),
                ("delete_pending", ctypes.c_ubyte), ("directory", ctypes.c_ubyte)]


def allocated(path: Path) -> int:
    h = k32.CreateFileW(str(path), 0x80, 7, None, 3, 0x02000000, None)  # FILE_READ_ATTRIBUTES, share all
    if h == wt.HANDLE(-1).value:
        raise OSError(ctypes.get_last_error(), str(path))
    try:
        info = FileStandardInfo()
        if not k32.GetFileInformationByHandleEx(h, 1, ctypes.byref(info), ctypes.sizeof(info)):
            raise OSError(ctypes.get_last_error(), str(path))
        return info.alloc
    finally:
        k32.CloseHandle(h)


def category(rel: str) -> str:
    parts = rel.split("/")
    if parts[0] != "maps":
        return "root settings.json" if rel == "settings.json" else "webapp (other)"
    sub = "/".join(parts[2:])
    if sub.startswith("tiles/0/"):
        return "hires .prbm.gz"
    if sub.startswith("tiles/"):
        return f"lowres LOD{parts[3]} .png"
    if sub.startswith("rstate/regions"):
        return "rstate regions.dat"
    if sub.startswith("rstate/"):
        return "rstate " + sub.rsplit(".", 2)[-2] + ".dat"
    if sub.startswith("textures.json"):
        return "textures.json.gz"
    if sub.startswith("settings.json"):
        return "map settings.json"
    if sub.startswith("live/"):
        return "live json"
    return "map (other)"


def inventory(root: Path):
    rows = defaultdict(lambda: [0, 0, 0, 0])  # n, logical, allocated, n<4K
    dirs = 0
    for p in root.rglob("*"):
        if p.is_dir():
            dirs += p.relative_to(root).as_posix().startswith("maps")
            continue
        rel = p.relative_to(root).as_posix()
        r = rows[category(rel)]
        size = p.stat().st_size
        r[0] += 1
        r[1] += size
        r[2] += allocated(p)
        r[3] += size < 4096
    return rows, dirs


def report(root: Path):
    rows, dirs = inventory(root)
    print(f"\n## {root}  (dirs under maps/: {dirs})")
    print("| category | files | logical B | allocated B | alloc/logical | files <4K |")
    print("|---|---|---|---|---|---|")
    tot = [0, 0, 0, 0]
    maps_tot = [0, 0, 0, 0]
    for cat in sorted(rows, key=lambda c: -rows[c][1]):
        n, lg, al, small = rows[cat]
        print(f"| {cat} | {n} | {lg:,} | {al:,} | {al / max(lg, 1):.3f} | {small} |")
        for i, v in enumerate(rows[cat]):
            tot[i] += v
            if not cat.startswith("webapp"):
                maps_tot[i] += v
    print(f"| **map data total** | {maps_tot[0]} | {maps_tot[1]:,} | {maps_tot[2]:,} | {maps_tot[2] / maps_tot[1]:.3f} | {maps_tot[3]} |")


def compare(a: Path, b: Path):
    print(f"\n## per-file {a.name} vs {b.name}")
    for label, glob, decode in (("hires", "maps/*/tiles/0/**/*.prbm.gz", gzip.decompress), ("lowres", "maps/*/tiles/[123]/**/*.png", None)):
        fa = {p.relative_to(a).as_posix(): p for p in a.glob(glob)}
        fb = {p.relative_to(b).as_posix(): p for p in b.glob(glob)}
        common = sorted(fa.keys() & fb.keys())
        sa = sb = same = same_raw = 0
        for k in common:
            x, y = fa[k].read_bytes(), fb[k].read_bytes()
            sa += len(x)
            sb += len(y)
            same += x == y
            if decode:
                same_raw += decode(x) == decode(y)
            else:
                from PIL import Image
                ia, ib = Image.open(io.BytesIO(x)), Image.open(io.BytesIO(y))
                same_raw += ia.convert("RGBA").tobytes() == ib.convert("RGBA").tobytes()
        print(f"{label}: only-A {len(fa.keys() - fb.keys())}, only-B {len(fb.keys() - fa.keys())}, common {len(common)}; "
              f"bytes A {sa:,} B {sb:,} (B/A {sb / max(sa, 1):.4f}); byte-identical {same}; content-identical {same_raw}")


for arg in sys.argv[1:]:
    if "=" in arg:
        x, y = arg.split("=")
        compare(Path(x), Path(y))
    else:
        report(Path(arg))
