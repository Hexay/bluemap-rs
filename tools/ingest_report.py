"""Analysis side of tools/ingest_bench.py: bluemap's log and final state -> latency, amplification, correctness.

A task's regions are recovered from the log by replaying `RenderQueue`: `Map 'x': N regions` finished a task made of
the first N distinct regions scheduled and not yet taken (`take` absorbs every queued region update).
"""
import gzip
import hashlib
import re
import struct
from bisect import bisect_left
from pathlib import Path

SCHEDULED = re.compile(r"Scheduled update for region-file: \((-?\d+), (-?\d+)\)")
FINISHED = re.compile(r"Map '[^']+': (\d+) regions, (\d+) tiles rendered, (\d+) skipped, (\d+) deleted, (\d+) lowres")
TILE_FILE = re.compile(r"^x(-?\d+)z(-?\d+)\.tiles\.dat$")
BUNDLE_FILE = re.compile(r"x(-?\d+)z(-?\d+)\.bmb$")
TILE_SIZE, TILE_OFFSET = 32, 2


def percentile(values, q: float) -> float:
    if not values:
        return float("nan")
    return sorted(values)[min(len(values) - 1, int(q * len(values)))]


def spread(values) -> dict:
    return {"p50": round(percentile(values, 0.5), 1), "p99": round(percentile(values, 0.99), 1),
            "mean": round(sum(values) / len(values), 1) if values else None}


class Log:
    """Timestamped bluemap output, from `t0` on (an earlier startup update isn't part of the crawl)."""

    def __init__(self, lines: list[tuple[float, str]], t0: float):
        self.events: list[tuple[float, tuple[int, int]]] = []
        self.tasks: list[dict] = []
        self.warnings: list[str] = []
        for t, line in lines:
            if re.search(r"\b(WARN|WARNING|ERROR)\b", line) and t >= t0:
                self.warnings.append(line)
            if t < t0:
                continue
            if m := SCHEDULED.search(line):
                self.events.append((t, (int(m[1]), int(m[2]))))
            elif m := FINISHED.search(line):
                self.tasks.append({"t": t, "regions": int(m[1]), "rendered": int(m[2]), "skipped": int(m[3]),
                                   "deleted": int(m[4]), "lowres": int(m[5])})
        self.mismatched = 0
        self.per_region: dict[tuple[int, int], list[tuple[float, float, float]]] = {}
        self._replay()

    def _replay(self) -> None:
        """Gives each schedule event its task's start and end."""
        next_event, prev_end = 0, self.tasks[0]["t"] if self.tasks else 0.0
        for task in self.tasks:
            taken, consumed = set(), []
            while next_event < len(self.events) and len(taken) < task["regions"]:
                t, r = self.events[next_event]
                if t > task["t"]:
                    break
                taken.add(r)
                consumed.append((t, r))
                next_event += 1
            # re-schedules of a region still queued are dropped by the queue: same task
            while next_event < len(self.events) and consumed:
                t, r = self.events[next_event]
                if t > prev_end or r not in taken:
                    break
                consumed.append((t, r))
                next_event += 1
            self.mismatched += len(taken) != task["regions"]
            start = max(prev_end, consumed[0][0]) if consumed else prev_end
            for t, r in consumed:
                self.per_region.setdefault(r, []).append((t, start, task["t"]))
            task["start"] = start
            prev_end = task["t"]
        self.unfinished = len(self.events) - next_event
        for t, r in self.events[next_event:]:
            self.per_region.setdefault(r, []).append((t, float("inf"), float("inf")))

    def picked_up(self, region, written: float):
        """(scheduled, task start, task end) of the update that saw a version written at `written`, or None."""
        events = self.per_region.get(region)
        if not events:
            return None
        i = bisect_left(events, (written, 0.0, 0.0))
        return events[i] if i < len(events) else None


def pipeline(versions, log: Log, t_end: float) -> dict:
    """Latency of each chunk through the pipeline and the backlog over the run; unfinished ones count up to t_end."""
    debounce_store, debounce_watch, queue, render, total, written_to_visible = [], [], [], [], [], []
    lag_series, unfinished_chunks = [], 0
    for v in versions:
        hit = log.picked_up(v.region, v.written)
        weight = len(v.offered) + v.restamped
        sched, start, end = hit if hit else (t_end, float("inf"), float("inf"))
        finished = end != float("inf")
        if not finished:
            unfinished_chunks += weight
            start = end = t_end
        lag_series.append((v.written, end - sched))
        for offered in v.offered:
            debounce_store.append(v.written - offered)
            total.append(end - offered)
        for _ in range(weight):
            debounce_watch.append(sched - v.written)
            queue.append(start - sched)
            render.append(end - start)
            written_to_visible.append(end - v.written)
    return {"store_debounce_s": spread(debounce_store), "watch_debounce_s": spread(debounce_watch),
            "queue_wait_s": spread(queue), "task_s": spread(render), "offered_to_visible_s": spread(total),
            "written_to_visible_s": spread(written_to_visible), "unfinished_chunks": unfinished_chunks,
            "lag_series": lag_series}


def bounded(versions, log: Log, t0: float, t_end: float, rate: float) -> tuple[bool, float, float]:
    """Backlog = chunks in written region files whose update hasn't finished, time-averaged per quarter of the run.
    Bounded: the last quarter's isn't above 1.3x the second's, or is under 30 s of offered chunks."""
    deltas = []
    for v in versions:
        weight = len(v.offered) + v.restamped
        deltas.append((v.written, weight))
        hit = log.picked_up(v.region, v.written)
        if hit and hit[2] != float("inf"):
            deltas.append((hit[2], -weight))
    deltas.sort()
    span = t_end - t0
    edges = [t0 + span * q / 4 for q in range(5)]
    area, level, prev, i = [0.0] * 4, 0, t0, 0
    for t, d in deltas + [(t_end, 0)]:
        t = min(t, t_end)
        while i < 4 and t >= edges[i + 1]:
            area[i] += level * (edges[i + 1] - prev)
            prev = edges[i + 1]
            i += 1
        if i < 4:
            area[i] += level * (t - prev)
            prev = t
        level += d
    q2, q4 = area[1] / (span / 4), area[3] / (span / 4)
    return q4 <= max(1.3 * q2, 30.0 * rate), q2, q4


def read_nbt(data: bytes) -> dict:
    """A gzip'd render-state cell: root compound of int arrays, or compounds of a string list and a byte array."""
    pos = 3 + struct.unpack_from(">H", data, 1)[0]

    def value(tag: int):
        nonlocal pos
        if tag == 10:
            out = {}
            while data[pos] != 0:
                t = data[pos]
                n = struct.unpack_from(">H", data, pos + 1)[0]
                name = data[pos + 3:pos + 3 + n].decode()
                pos += 3 + n
                out[name] = value(t)
            pos += 1
            return out
        if tag == 11:
            n = struct.unpack_from(">i", data, pos)[0]
            pos += 4 + 4 * n
            return struct.unpack_from(f">{n}i", data, pos - 4 * n)
        if tag == 7:
            n = struct.unpack_from(">i", data, pos)[0]
            pos += 4 + n
            return data[pos - n:pos]
        if tag == 9:
            elem = data[pos]
            n = struct.unpack_from(">i", data, pos + 1)[0]
            pos += 5
            return [value(elem) for _ in range(n)]
        if tag == 8:
            n = struct.unpack_from(">H", data, pos)[0]
            pos += 2 + n
            return data[pos - n:pos].decode()
        raise ValueError(f"nbt tag {tag}")

    return value(10)


def tile_states(map_dir: Path) -> dict[tuple[int, int], tuple[str, int]]:
    """Every known tile's (state, last render time) from rstate."""
    out = {}
    for path in (map_dir / "rstate").rglob("*.tiles.dat"):
        m = TILE_FILE.search(path.relative_to(map_dir / "rstate").as_posix().replace("/", ""))  # digit-split paths
        cell = read_nbt(gzip.decompress(path.read_bytes()))
        times, states = cell["last-render-times"], cell["tile-states"]
        palette, data = states["palette"], states["data"]
        cx, cz = int(m[1]), int(m[2])
        for i, index in enumerate(data):
            state = palette[index]
            if state != "bluemap:unknown":
                out[(cx << 5) | (i & 31), (cz << 5) | (i >> 5)] = (state.removeprefix("bluemap:"), times[i])
    return out


def chunks_under(tile: tuple[int, int]):
    x, z = tile[0] * TILE_SIZE + TILE_OFFSET, tile[1] * TILE_SIZE + TILE_OFFSET
    return [(cx, cz) for cz in range(z >> 4, ((z + TILE_SIZE - 1) >> 4) + 1)
            for cx in range(x >> 4, ((x + TILE_SIZE - 1) >> 4) + 1)]


def tile_latency(states: dict, offered: dict, stamps) -> dict:
    """For each final rendered tile: last chunk under it offered, or last rewritten, -> the tile's last render start."""
    from_offer, from_write = [], []
    for tile, (state, rendered) in states.items():
        if not state.startswith("rendered"):
            continue
        chunks = chunks_under(tile)
        offers = [offered[c] for c in chunks if c in offered]
        if len(offers) != len(chunks):
            continue
        from_offer.append(rendered - max(offers))
        last = max((stamps(c) for c in chunks), default=0)
        from_write.append(rendered - last)
    return {"tile_complete_to_render_start_s": spread(from_offer), "last_write_to_render_start_s": spread(from_write),
            "tiles": len(from_offer)}


def bundle_tiles(path: Path) -> tuple[dict[tuple[int, int], bytes], int, int]:
    """(tile -> blob digest), live bytes, file bytes of a hires bundle (bundle/log.rs: last record wins)."""
    data = path.read_bytes()
    live, pos = {}, 16
    while pos + 12 <= len(data):
        kind, lx, lz, _, length, _ = struct.unpack_from("<BBBBII", data, pos)
        end = pos + 12 + length
        if kind not in (1, 2) or end > len(data):
            break
        if kind == 1:
            live[lx, lz] = (hashlib.blake2b(data[pos + 12:end], digest_size=16).digest(), length)
        else:
            live.pop((lx, lz), None)
        pos = end
    return {k: v[0] for k, v in live.items()}, sum(v[1] + 12 for v in live.values()) + 16, len(data)


def hires_digests(map_dir: Path) -> tuple[dict, int, int]:
    """Whole-map tile digests by world tile coordinates; live and file bytes."""
    tiles, live, size = {}, 0, 0
    for path in (map_dir / "hires").glob("*.bmb"):
        m = BUNDLE_FILE.search(path.name)
        bx, bz = int(m[1]), int(m[2])
        bundle, l, s = bundle_tiles(path)
        live, size = live + l, size + s
        for (lx, lz), digest in bundle.items():
            tiles[(bx << 4) | lx, (bz << 4) | lz] = digest
    return tiles, live, size


def lowres_files(map_dir: Path) -> dict[str, bytes]:
    root = map_dir / "tiles"
    return {p.relative_to(root).as_posix(): hashlib.blake2b(p.read_bytes(), digest_size=16).digest()
            for p in root.rglob("*") if p.is_file()}


def compare_maps(clean: Path, crawl: Path) -> dict:
    """Crawl output against a clean `-r -f` of the same world: hires blobs, lowres files, tile states."""
    a_hires, a_live, a_size = hires_digests(clean)
    b_hires, b_live, b_size = hires_digests(crawl)
    missing = sorted(a_hires.keys() - b_hires.keys())
    extra = sorted(b_hires.keys() - a_hires.keys())
    differ = sorted(t for t in a_hires.keys() & b_hires.keys() if a_hires[t] != b_hires[t])
    a_low, b_low = lowres_files(clean), lowres_files(crawl)
    low_differ = sorted(f for f in a_low.keys() & b_low.keys() if a_low[f] != b_low[f])
    a_state, b_state = tile_states(clean), tile_states(crawl)
    states_differ = sum(1 for t in a_state.keys() | b_state.keys()
                        if a_state.get(t, ("-",))[0] != b_state.get(t, ("-",))[0])
    return {"hires_clean": len(a_hires), "hires_crawl": len(b_hires), "hires_missing": len(missing),
            "hires_extra": len(extra), "hires_differ": len(differ), "missing_sample": missing[:10],
            "extra_sample": extra[:10], "differ_sample": differ[:10],
            "lowres_clean": len(a_low), "lowres_crawl": len(b_low), "lowres_missing": len(a_low.keys() - b_low.keys()),
            "lowres_extra": len(b_low.keys() - a_low.keys()), "lowres_differ": len(low_differ),
            "lowres_differ_sample": low_differ[:10], "tile_states_differ": states_differ,
            "bundle_live_bytes": b_live, "bundle_file_bytes": b_size, "clean_bundle_bytes": a_size,
            "identical": not (missing or extra or differ or low_differ or states_differ
                              or a_low.keys() != b_low.keys())}
