"""Write a minimal lit 26.x world (8x8 chunks of stone under grass, no level.dat) for smoke tests.

    python3 tools/tiny_world.py <world dir>

Stdlib only, so it runs on bare CI runners (tools/docker_smoke.sh).
"""
import struct
import sys
import time
import zlib
from pathlib import Path

DATA_VERSION = 5023  # 26.3
SURFACE_SECTION = 4  # blocks 64..79: above remove-caves-below-y, so no heightmaps needed


def tag(kind: int, name: str, payload: bytes) -> bytes:
    raw = name.encode()
    return bytes([kind]) + struct.pack(">H", len(raw)) + raw + payload


def string(s: str) -> bytes:
    raw = s.encode()
    return struct.pack(">H", len(raw)) + raw


def compound(*tags: bytes) -> bytes:
    return b"".join(tags) + b"\x00"


def compound_list(name: str, items: list[bytes]) -> bytes:
    return tag(9, name, bytes([10]) + struct.pack(">i", len(items)) + b"".join(items))


def pack(indices: list[int], bits: int) -> bytes:
    per_long, longs = 64 // bits, []
    for start in range(0, len(indices), per_long):
        value = sum(i << (bits * n) for n, i in enumerate(indices[start:start + per_long]))
        longs.append(value - (1 << 64) if value >= 1 << 63 else value)
    return struct.pack(">i", len(longs)) + b"".join(struct.pack(">q", v) for v in longs)


def section(y: int, palette: list[str], indices: list[int] | None) -> bytes:
    states = compound_list("palette", [compound(tag(8, "Name", string(f"minecraft:{p}"))) for p in palette])
    if indices:
        states += tag(12, "data", pack(indices, 4))
    biomes = tag(9, "palette", bytes([8]) + struct.pack(">i", 1) + string("minecraft:plains"))
    sky = tag(7, "SkyLight", struct.pack(">i", 2048) + b"\xff" * 2048)
    return compound(tag(1, "Y", bytes([y])), tag(10, "block_states", compound(states)),
                    tag(10, "biomes", compound(biomes)), sky)


def chunk(cx: int, cz: int) -> bytes:
    # index = (y * 16 + z) * 16 + x; layers 0-2 stone, 3 grass, the rest air
    surface = [0 if i < 768 else 1 if i < 1024 else 2 for i in range(4096)]
    sections = [section(SURFACE_SECTION, ["stone", "grass_block", "air"], surface),
                section(SURFACE_SECTION + 1, ["air"], None)]
    root = compound(tag(3, "DataVersion", struct.pack(">i", DATA_VERSION)), tag(8, "Status", string("minecraft:full")),
                    tag(3, "xPos", struct.pack(">i", cx)), tag(3, "zPos", struct.pack(">i", cz)),
                    tag(3, "yPos", struct.pack(">i", -4)), compound_list("sections", sections))
    return tag(10, "", root)


def region(chunks: dict[tuple[int, int], bytes]) -> bytes:
    header, body = bytearray(8192), bytearray()
    for (cx, cz), nbt in chunks.items():
        data = zlib.compress(nbt)
        blob = struct.pack(">IB", len(data) + 1, 2) + data
        blob += b"\x00" * (-len(blob) % 4096)
        offset = 2 + len(body) // 4096
        struct.pack_into(">I", header, 4 * (cx + cz * 32), offset << 8 | len(blob) // 4096)
        # renders skip chunks whose timestamp is not newer than the tile's last render (0 for a new map)
        struct.pack_into(">I", header, 4096 + 4 * (cx + cz * 32), int(time.time()))
        body += blob
    return bytes(header + body)


def main() -> None:
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    out = Path(sys.argv[1]) / "dimensions/minecraft/overworld/region"
    out.mkdir(parents=True, exist_ok=True)
    (out / "r.0.0.mca").write_bytes(region({(x, z): chunk(x, z) for x in range(8) for z in range(8)}))


if __name__ == "__main__":
    main()
