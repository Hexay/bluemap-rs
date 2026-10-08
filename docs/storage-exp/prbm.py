"""PRBM v1 (BlueMap 5.27) parse/write with numpy, plus corpus discovery. See docs/03-rendering.md §3."""
import glob
import gzip
import os
import struct

import numpy as np

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "work", "bluemap")
ATTRS = [  # name, flag byte, dtype, cardinality
    ("position", 0x21, "<f4", 3),
    ("normal", 0x63, "i1", 3),
    ("color", 0x67, "u1", 3),
    ("uv", 0x11, "<f4", 2),
    ("ao", 0x47, "u1", 1),
    ("blocklight", 0x03, "i1", 1),
    ("sunlight", 0x03, "i1", 1),
]
ENC = {1: "<f4", 3: "i1", 7: "u1"}


def corpus():
    """[(map_id, path)] for every hires tile, sorted."""
    out = []
    for p in sorted(glob.glob(f"{ROOT}/*/web/maps/*/tiles/0/**/*.prbm.gz", recursive=True)):
        p = p.replace("\\", "/")
        world = p[len(ROOT) + 1:].split("/")[0]
        mapid = p.split("/web/maps/")[1].split("/")[0]
        out.append((mapid if world == mapid.replace("_", "-") else f"{world}/{mapid}", p))
    return out


def load(path):
    with open(path, "rb") as f:
        gz = f.read()
    return gz, gzip.decompress(gz)


def pad4(n):
    return (-n) % 4


def parse(buf):
    """-> dict(attr name -> ndarray[N, card]), groups ndarray[G,3] int32, n vertices."""
    assert buf[0] == 1 and buf[1] == 7, (buf[0], buf[1])
    n = int.from_bytes(buf[2:5], "little")
    assert buf[5:8] == b"\0\0\0"
    pos = 8
    t = {}
    for name, flag, dt, card in ATTRS:
        end = buf.index(b"\0", pos)
        assert buf[pos:end].decode() == name
        pos = end + 1
        assert buf[pos] == flag and ENC[flag & 0xF] == dt and ((flag >> 4) & 3) + 1 == card
        pos += 1
        pos += pad4(pos)
        size = np.dtype(dt).itemsize * n * card
        t[name] = np.frombuffer(buf, dt, n * card, pos).reshape(n, card)
        pos += size
    pos += pad4(pos)
    rest = np.frombuffer(buf, "<i4", (len(buf) - pos) // 4, pos)
    assert rest[-1] == -1 and (len(rest) - 1) % 3 == 0 and len(buf) == pos + 4 * len(rest)
    t["groups"] = rest[:-1].reshape(-1, 3)
    t["n"] = n
    return t


def write(t):
    """Exact inverse of parse(): PRBMWriter byte layout."""
    n = t["n"]
    out = bytearray(b"\x01\x07" + n.to_bytes(3, "little") + b"\0\0\0")
    for name, flag, dt, card in ATTRS:
        out += name.encode() + b"\0" + bytes([flag])
        out += b"\0" * pad4(len(out))
        out += np.ascontiguousarray(t[name], dtype=dt).tobytes()
    out += b"\0" * pad4(len(out))
    out += np.ascontiguousarray(t["groups"], dtype="<i4").tobytes() + struct.pack("<i", -1)
    return bytes(out)


def attr_bytes(n):
    """Payload bytes per attribute for n vertices."""
    return {name: np.dtype(dt).itemsize * n * card for name, _, dt, card in ATTRS}


def lowres_pngs():
    return sorted(p.replace("\\", "/") for p in glob.glob(f"{ROOT}/*/web/maps/*/tiles/[1-9]/**/*.png", recursive=True))


def cluster(size, c=4096):
    return (size + c - 1) // c * c


def disk_size(path):
    return os.path.getsize(path)
