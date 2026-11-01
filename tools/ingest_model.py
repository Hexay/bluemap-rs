"""The writer side of tools/ingest_bench.py: `acacia-anvil`'s AnvilStore (crates/acacia-anvil/src/store in the acacia
repo) and bot crawl (examples/scale/bots.rs) replayed with real chunks instead of converted ones. Conversion, light
and compression are not simulated; the region write pattern, debounce, timestamps and rename are.

Differences from the real store, all on the safe side for the renderer: a column relit because a neighbour arrived
always counts as changed (the real store keeps its timestamp when the NBT is byte-identical), unless
`relight_changes` is off.
"""
import os
import random
import struct
import time
from array import array
from collections import defaultdict, deque
from dataclasses import dataclass, field
from pathlib import Path

SECTOR = 4096
CORE_SPAN = 8  # fixtures/real has 8x8 fully generated regions, -4..3 (as tools/scale_world.py)
VIEW_RADIUS = 8
STRIDE = 15  # new columns per step once passes overlap; bots.rs Field.stride
BLOCK_SIDE = 128  # chunks a scattered bot mows before moving on; 4x4 regions
NEIGHBOURS = [(dx, dz) for dz in (-1, 0, 1) for dx in (-1, 0, 1)]


def region_of(pos: tuple[int, int]) -> tuple[int, int]:
    return pos[0] >> 5, pos[1] >> 5


class Source:
    """Real chunks by slot. A chunk at (cx, cz) comes from the same slot of the core region its region tiles onto."""

    def __init__(self, region_dir: Path):
        self.slots = {}
        self.chunks = self.bytes = 0
        for sz in range(-CORE_SPAN // 2, CORE_SPAN // 2):
            for sx in range(-CORE_SPAN // 2, CORE_SPAN // 2):
                view = memoryview((region_dir / f"r.{sx}.{sz}.mca").read_bytes())
                table = []
                for i in range(1024):
                    loc = int.from_bytes(view[i * 4:i * 4 + 4], "big")
                    off, count = (loc >> 8) * SECTOR, loc & 0xFF
                    table.append(view[off:off + count * SECTOR] if count else None)
                    if count:
                        self.chunks += 1
                        self.bytes += count * SECTOR
                self.slots[sx, sz] = table

    def sectors(self, rx: int, rz: int, slot: int) -> memoryview:
        half = CORE_SPAN // 2
        return self.slots[rx % CORE_SPAN - half, rz % CORE_SPAN - half][slot]


@dataclass
class Version:
    """One rewrite of a region file."""
    region: tuple[int, int]
    written: float
    offered: array  # when each chunk new in this version reached the store
    restamped: int
    size: int


@dataclass
class RegionFile:
    stamps: dict[int, int] = field(default_factory=dict)
    last_ready: float = 0.0


class Store:
    """AnvilStore.update + flush. Everything seen stays a known neighbour (the real store reads evicted ones back)."""

    def __init__(self, region_dir: Path, source: Source, max_wait=10.0, interval=10.0, hold=60.0, relight_changes=True):
        self.dir, self.source = region_dir, source
        self.max_wait, self.interval, self.hold = max_wait, interval, hold
        self.relight_changes = relight_changes
        self.offered: dict[tuple[int, int], float] = {}
        self.present: dict[tuple[int, int], int] = {}
        self.dirty: dict[tuple[int, int], float] = {}
        self.waiting: deque = deque()
        self.ready: dict[tuple[int, int], dict] = defaultdict(dict)
        self.first_ready: dict[tuple[int, int], float] = {}
        self.touched: dict[tuple[int, int], float] = {}
        self.written: dict[tuple[int, int], float] = {}
        self.regions: dict[tuple[int, int], RegionFile] = {}
        self.versions: list[Version] = []
        self.bytes_written = 0
        self.completed: list[tuple[float, tuple[int, int]]] = []

    def _mark_ready(self, pos, since) -> None:
        r = region_of(pos)
        self.ready[r][pos] = since
        self.first_ready[r] = min(self.first_ready.get(r, since), since)

    def update(self, pos: tuple[int, int], now: float) -> bool:
        if pos in self.offered:
            return False  # sent again unchanged: `update` dirties nothing
        self.offered[pos] = now
        for dx, dz in NEIGHBOURS:
            p = (pos[0] + dx, pos[1] + dz)
            n = self.present.get(p, 0) + 1
            if n == 9:
                self.present.pop(p, None)
            else:
                self.present[p] = n
            self.touched[region_of(p)] = now
            if p in self.offered:
                if p not in self.dirty:
                    self.dirty[p] = now
                    self.waiting.append((now, p))
                if n == 9:
                    self._mark_ready(p, self.dirty[p])
        return True

    def flush(self, now: float) -> int:
        while self.waiting and now - self.waiting[0][0] >= self.max_wait:
            since, p = self.waiting.popleft()
            if self.dirty.get(p) == since:
                self._mark_ready(p, since)
        quiet = lambda t: t is None or now - t >= self.interval
        writes = 0
        for r in [r for r in self.ready if self.ready[r]]:
            if quiet(self.written.get(r)) and (quiet(self.touched.get(r)) or now - self.first_ready[r] >= self.hold):
                self._write_region(r, self.ready.pop(r), now)
                del self.first_ready[r]
                self.written[r] = now
                writes += 1
        self.touched = {r: t for r, t in self.touched.items() if now - t < self.interval}
        self.written = {r: t for r, t in self.written.items() if now - t < self.interval}
        return writes

    def idle(self) -> bool:
        return not self.dirty

    def _write_region(self, r, cols: dict, now: float) -> None:
        region = self.regions.setdefault(r, RegionFile())
        was_full = len(region.stamps) == 1024
        secs = int(time.time())
        offered, restamped = array("d"), 0
        for pos in cols:
            del self.dirty[pos]
            slot = (pos[0] & 31) | ((pos[1] & 31) << 5)
            old = region.stamps.get(slot)
            if old is None:
                region.stamps[slot] = secs
                offered.append(self.offered[pos])
            elif self.relight_changes:
                region.stamps[slot] = max(secs, old + 1)
                restamped += 1
        if not offered and not restamped:
            return
        size = self._save(r, region)
        self.versions.append(Version(r, time.time(), offered, restamped, size))
        self.bytes_written += size
        if not was_full and len(region.stamps) == 1024:
            self.completed.append((time.time(), r))

    def _save(self, r, region: RegionFile) -> int:
        """Region.encode + Region.save: whole file in slot order, temp sibling, rename."""
        locs, stamps, body = bytearray(SECTOR), bytearray(SECTOR), []
        sector = 2
        for slot in sorted(region.stamps):
            data = self.source.sectors(r[0], r[1], slot)
            count = len(data) // SECTOR
            struct.pack_into(">I", locs, slot * 4, (sector << 8) | count)
            struct.pack_into(">I", stamps, slot * 4, region.stamps[slot])
            body.append(data)
            sector += count
        path = self.dir / f"r.{r[0]}.{r[1]}.mca"
        tmp = path.with_name(path.name + ".tmp")
        with open(tmp, "wb") as f:
            f.writelines([locs, stamps, *body])
        os.rename(tmp, path)
        return sector * SECTOR


def view_delta(at, frm, inside=None) -> list[tuple[int, int]]:
    """Chunks in the square view at `at` that were not in the one at `frm`; `frm` None sends the whole view."""
    r = VIEW_RADIUS
    out = []
    for z in range(at[1] - r, at[1] + r + 1):
        for x in range(at[0] - r, at[0] + r + 1):
            if frm is not None and abs(x - frm[0]) <= r and abs(z - frm[1]) <= r:
                continue
            if inside is None or inside(x, z):
                out.append((x, z))
    return out


class Spiral:
    """One compact frontier: square spiral `STRIDE` chunks apart, at `speed` chunks per second."""

    def __init__(self, origin: tuple[int, int], speed: float):
        self.at, self.speed = origin, speed
        self.carry, self.dir, self.leg, self.left, self.legs_done = 0.0, 0, STRIDE, STRIDE, 0
        self.started = False

    def advance(self, dt: float, out: list) -> None:
        if not self.started:
            self.started = True
            out.extend(view_delta(self.at, None))
        self.carry += self.speed * dt
        steps = [(1, 0), (0, 1), (-1, 0), (0, -1)]
        while self.carry >= 1.0:
            self.carry -= 1.0
            frm = self.at
            self.at = (frm[0] + steps[self.dir][0], frm[1] + steps[self.dir][1])
            out.extend(view_delta(self.at, frm))
            self.left -= 1
            if self.left == 0:
                self.dir = (self.dir + 1) % 4
                self.legs_done += 1
                if self.legs_done % 2 == 0:
                    self.leg += STRIDE
                self.left = self.leg


def scatter_blocks(origin: tuple[int, int], count: int, seed: int) -> list[tuple[int, int]]:
    """`count` region-aligned blocks, shuffled, spread over about 4x their area so that most don't touch."""
    grid = 2
    while grid * grid < count * 4:
        grid += 1
    cells = [(origin[0] + i * BLOCK_SIDE, origin[1] + j * BLOCK_SIDE) for j in range(grid) for i in range(grid)]
    random.Random(seed).shuffle(cells)
    return cells[:count]


class Mower:
    """A bot mowing one block in passes along x, then moving on to the next free block (bots.rs Field.advance)."""

    def __init__(self, blocks: list, speed: float, rng: random.Random):
        self.blocks, self.speed, self.rng = blocks, speed, rng
        self.carry, self.done = rng.random(), False
        self.block = None
        self._next_block(first=True)
        self.pending_view = True

    def _inside(self, x: int, z: int) -> bool:
        x0, z0 = self.block
        return x0 <= x < x0 + BLOCK_SIDE and z0 <= z < z0 + BLOCK_SIDE

    def _next_block(self, first=False) -> None:
        if not self.blocks:
            self.done = True
            return
        self.block = self.blocks.pop()
        x0, z0 = self.block
        passes = BLOCK_SIDE // STRIDE
        # bots don't all start at a block's corner in step
        start_pass = self.rng.randrange(passes) if first else 0
        self.at = (x0, z0 + VIEW_RADIUS + start_pass * STRIDE)
        self.dir, self.turn, self.pending_view = 1, 0, True

    def advance(self, dt: float, out: list) -> None:
        self.carry += self.speed * dt
        while self.carry >= 1.0 and not self.done:
            if self.pending_view:
                out.extend(view_delta(self.at, None, self._inside))
                self.pending_view = False
            self.carry -= 1.0
            frm = self.at
            x = frm[0] + self.dir
            if self.turn > 0:
                self.at = (frm[0], frm[1] + 1)
                self.turn -= 1
                if self.turn == 0:
                    self.dir = -self.dir
            elif self.block[0] <= x < self.block[0] + BLOCK_SIDE:
                self.at = (x, frm[1])
            elif frm[1] - VIEW_RADIUS + STRIDE >= self.block[1] + BLOCK_SIDE:
                self._next_block()
                continue
            else:
                self.at = (frm[0], frm[1] + 1)
                self.turn = STRIDE - 1
            out.extend(view_delta(self.at, frm, self._inside))
