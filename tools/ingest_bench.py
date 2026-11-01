"""Sustained new-chunk throughput of `bluemap -u` into `format: optimized`, the DonutSMP deployment: real chunks of the
4096² world replayed into an empty world the way acacia-anvil writes them (tools/ingest_model.py), with the default
watcher settings. Results: docs/results/donutsmp-ingest-throughput-2026-11-01.md. Linux (devbox), stdlib only.

Usage: python3 tools/ingest_bench.py run <label> --pattern spiral|scatter --rate R [--threads 12] [--duration 600]
           [--settle] [--compare] [--exe P] [--large] [--keep]
       python3 tools/ingest_bench.py sweep <label> --pattern P --start R [--threads 12] [--rung 240] [--final 600]

--rate R: spiral: one strip at R/15 chunks/s (15 new columns a step). scatter: R/21 bots at 1.4 chunks/s (bots.rs) unless
--bots/--speed say otherwise; a bot really yields about 19 new chunks/s, so read the offered rate from the summary, not
R. --large: over a copy of the k=8 (32768²) map, crawling beyond its edge. Each run holds ~/bench.lock and won't start,
or stops crawling, under 10 GB free on disk. World goes to /dev/shm (RAM, 9.1 MB a region), crawl output to disk; both are
deleted unless --keep. tmpfs caps 10 min runs near 3800 chunks/s. Appends one summary per run to <--out>/results.jsonl.
"""
import argparse
import fcntl
import json
import os
import random
import re
import shutil
import threading
import time
from pathlib import Path

from ingest_model import BLOCK_SIDE, Mower, Source, Spiral, Store, scatter_blocks
from ingest_proc import Bluemap
from ingest_report import Log, bounded, compare_maps, pipeline, spread, tile_latency, tile_states

HOME = Path.home()
FIXTURES = HOME / "bmrs-devbench" / "work"
SOURCE_WORLD = FIXTURES / "worlds" / "real" / "world"
REGION = Path("dimensions/minecraft/overworld/region")
CONFIG_FROM = FIXTURES / "real" / "d12-opt" / "config"
DATA_FROM = FIXTURES / "scale" / "8-optimized" / "data"
LARGE_MAP = FIXTURES / "scale" / "8-optimized" / "web" / "maps" / "real"
LARGE_WORLD = FIXTURES / "scale" / "world-8" / REGION
LARGE_ORIGIN = (34 * 32, 0)  # chunks; the k=8 map ends at region 31, so no existing bundle gets appended to
SHM = Path("/dev/shm/bmrs-ingest")
TICK = 0.5  # crawl.rs ingest loop
MIN_FREE_DISK, MIN_FREE_SHM = 10 << 30, 3 << 30  # the disk is shared: a run never takes it below 10 GB
NEW_PER_STEP = 15


def free_bytes(path: Path) -> int:
    s = os.statvfs(path)
    return s.f_bavail * s.f_frsize


def set_conf(path: Path, key: str, value: str) -> None:
    text = path.read_text()
    text, n = re.subn(rf"(?m)^{key}: .*$", f"{key}: {value}", text)
    path.write_text(text if n else f"{key}: {value}\n{text}")


def tree_bytes(path: Path) -> int:
    return sum(p.stat().st_size for p in path.rglob("*") if p.is_file()) if path.exists() else 0


def prepare(base: Path, world: Path, threads: int, large: bool) -> None:
    shutil.rmtree(base, ignore_errors=True)
    (base / "data").mkdir(parents=True)
    shutil.copytree(CONFIG_FROM, base / "config")
    for name in ("minecraft-client-26.3.jar", "resourceExtensions.zip"):
        (base / "data" / name).symlink_to(DATA_FROM / name)
    set_conf(base / "config" / "core.conf", "render-thread-count", str(threads))
    set_conf(base / "config" / "maps" / "real.conf", "world", json.dumps(world.as_posix()))
    set_conf(base / "config" / "storages" / "file.conf", "format", "optimized")
    if large:
        map_dir = base / "web" / "maps" / "real"
        map_dir.parent.mkdir(parents=True)
        (map_dir.parent / "bluemap-rs-format.txt").write_text((LARGE_MAP.parent / "bluemap-rs-format.txt").read_text())
        shutil.copytree(LARGE_MAP / "hires", map_dir / "hires", copy_function=os.link)
        for item in ("rstate", "tiles", "live"):
            shutil.copytree(LARGE_MAP / item, map_dir / item)
        for item in ("settings.json", "textures.json.gz"):
            shutil.copy2(LARGE_MAP / item, map_dir / item)


def prepare_world(world: Path, large: bool) -> Path:
    shutil.rmtree(world, ignore_errors=True)
    region_dir = world / REGION
    region_dir.mkdir(parents=True)
    shutil.copy2(SOURCE_WORLD / "level.dat", world / "level.dat")
    if large:
        for mca in LARGE_WORLD.glob("r.*.mca"):
            (region_dir / mca.name).symlink_to(mca)
    return region_dir


def make_bots(a) -> tuple[list, str]:
    origin = LARGE_ORIGIN if a.large else (0, 0)
    if a.pattern == "spiral":
        speed = a.speed or a.rate / NEW_PER_STEP
        return [Spiral(origin, speed)], f"1 strip at {speed:.1f} chunks/s"
    speed = a.speed or 1.4
    n = a.bots or max(1, round(a.rate / (NEW_PER_STEP * speed)))
    per_bot = BLOCK_SIDE * BLOCK_SIDE / (NEW_PER_STEP * speed)
    count = n * (2 + int(a.duration / per_bot))
    blocks = scatter_blocks(origin, count, seed=1)
    rng = random.Random(2)
    return [Mower(blocks, speed, rng) for _ in range(n)], f"{n} bots at {speed} chunks/s"


def crawl(a, store: Store, bm: Bluemap) -> dict:
    bots, describe = make_bots(a)
    start = last = time.time()
    end = start + a.duration
    offered = overrun = 0
    while (now := time.time()) < end and bm.proc.poll() is None:
        fresh: list = []
        for bot in bots:
            bot.advance(now - last, fresh)
        last = now
        offered += sum(store.update(pos, now) for pos in fresh)
        store.flush(now)
        if free_bytes(HOME) < MIN_FREE_DISK or free_bytes(Path("/dev/shm")) < MIN_FREE_SHM:
            return {"describe": describe, "start": start, "end": now, "offered": offered, "aborted": "low disk/shm",
                    "overrun": overrun}
        spent = time.time() - now
        overrun = max(overrun, spent - TICK)
        time.sleep(max(0.0, TICK - spent))
    return {"describe": describe, "start": start, "end": time.time(), "offered": offered, "overrun": overrun}


def settle(store: Store, bm: Bluemap, limit: float) -> float:
    """Store flushed and nothing scheduled or running for a watcher cooldown: nothing is left in flight."""
    deadline = time.time() + limit
    while time.time() < deadline and not store.idle():
        store.flush(time.time())
        time.sleep(TICK)
    last_write = time.time()
    while time.time() < deadline:
        log = Log(bm.lines, last_write)
        last = max([t for t, _ in log.events] + [task["t"] for task in log.tasks] + [last_write])
        if not log.unfinished and time.time() - last >= 70:
            return last
        time.sleep(2)
    return -1.0


def samples_between(samples: list[dict], t0: float, t1: float) -> dict:
    window = [s for s in samples if t0 <= s["t"] <= t1]
    if len(window) < 2:
        return {}
    a, b = window[0], window[-1]
    span = b["t"] - a["t"]
    step = 5
    cores = [(q["cpu"] - p["cpu"]) / (q["t"] - p["t"]) for p, q in zip(window[::step], window[step::step])]
    rss = [s["rss"] for s in window]
    return {"cores_avg": round((b["cpu"] - a["cpu"]) / span, 2), "cores_peak_5s": round(max(cores, default=0), 2),
            "cpu_s": round(b["cpu"] - a["cpu"], 1), "rss_avg_mb": round(sum(rss) / len(rss) / 2**20),
            "rss_peak_sampled_mb": round(max(rss) / 2**20), "wchar": b["wchar"] - a["wchar"],
            "write_bytes": b["write_bytes"] - a["write_bytes"], "rchar": b["rchar"] - a["rchar"],
            "read_bytes": b["read_bytes"] - a["read_bytes"],
            "others_cores_avg": round((b["sys_busy"] - a["sys_busy"] - (b["cpu"] - a["cpu"])) / span, 2),
            "load_avg": round(sum(s["load"] for s in window) / len(window), 1)}


def clean_render(a, world: Path, threads: int) -> tuple[dict, Path]:
    """A forced render of the settled world, for correctness and its cost per chunk; on tmpfs, the disk is nearly full."""
    base = SHM / a.label / "clean"
    prepare(base, world, threads, False)
    start = time.time()
    bm = Bluemap(a.exe, base, ["-r", "-f"])
    _, _, usage = os.wait4(bm.proc.pid, 0)
    wall = time.time() - start
    return {"wall_s": round(wall, 1), "cpu_s": round(usage.ru_utime + usage.ru_stime, 1),
            "rss_peak_mb": round(usage.ru_maxrss / 1024),
            "final_tile_line": [l for _, l in bm.lines if "tiles rendered" in l][:1]}, base


def run(a) -> dict:
    base = Path(a.out) / a.label
    world = SHM / a.label / "world"
    SHM.mkdir(parents=True, exist_ok=True)
    source = Source(SOURCE_WORLD / REGION)
    lock = open(HOME / "bench.lock", "w")
    print(f"== {a.label}: waiting for the bench lock", flush=True)
    queued = time.time()
    fcntl.flock(lock, fcntl.LOCK_EX)
    print(f"== {a.label}: lock after {time.time() - queued:.0f}s, load {os.getloadavg()[0]:.1f}", flush=True)
    if free_bytes(HOME) < MIN_FREE_DISK:
        fcntl.flock(lock, fcntl.LOCK_UN)
        raise SystemExit(f"{a.label}: under {MIN_FREE_DISK >> 30} GB free on {HOME}, not starting")
    summary = {"label": a.label, "pattern": a.pattern, "rate": a.rate, "threads": a.threads, "duration": a.duration,
               "large": a.large, "relight_changes": not a.relight_unchanged,
               "source_chunks": source.chunks, "source_bytes_per_chunk": round(source.bytes / source.chunks)}
    try:
        region_dir = prepare_world(world, a.large)
        prepare(base, world, a.threads, a.large)
        store = Store(region_dir, source, relight_changes=not a.relight_unchanged)
        boot = time.time()
        bm = Bluemap(a.exe, base, ["-u"])
        bm.wait_for("Your maps are now all up-to-date!", 1800)
        bm.wait_for("Started watching map", 60)
        summary["startup_s"] = round(time.time() - boot, 1)
        out_before = tree_bytes(base / "web" / "maps" / "real" / "rstate")
        if a.perf:
            profile = Path(a.out) / f"{a.label}.perf.txt"
            threading.Timer(a.duration * 0.7, bm.perf, (a.perf, profile)).start()
        crawled = crawl(a, store, bm)
        summary["thread_cpu_s"] = bm.thread_cpu()
        summary.update({k: crawled[k] for k in ("describe", "offered", "overrun")}, aborted=crawled.get("aborted"))
        t0, t1 = crawled["start"], crawled["end"]
        settled = settle(store, bm, a.settle_limit) if a.settle else None
        t_end = time.time()
        cpu_total, rss_peak = bm.terminate()
        log = Log(bm.lines, t0)
        versions = store.versions
        span = t1 - t0
        p = pipeline(versions, log, t_end if a.settle else t1)
        p.pop("lag_series")
        ok, q2, q4 = bounded(versions, log, t0, t1, crawled["offered"] / span)
        done = sum(1 for t, _ in store.completed if t <= t1)
        rendered = sum(t["rendered"] for t in log.tasks)
        chunks = len(store.offered)
        run_window = samples_between(bm.samples, t0, t1)
        all_window = samples_between(bm.samples, t0, t_end)
        map_dir = base / "web" / "maps" / "real"
        summary.update({
            "offered_chunks_per_s": round(crawled["offered"] / span, 1), "regions_completed_per_s": round(done / span, 3),
            "regions_written_per_s": round(len(versions) / span, 3), "region_rewrites_per_region":
                round(len(versions) / max(1, len(store.regions)), 2),
            "chunks": chunks, "regions": len(store.regions), "regions_full": len(store.completed),
            "bounded": ok, "backlog_q2_chunks": round(q2), "backlog_q4_chunks": round(q4), **p,
            "settled": None if settled is None else settled > 0, "drain_s": round(t_end - t1, 1) if a.settle else None,
            "tasks": len(log.tasks), "regions_scheduled": len(log.events), "task_mismatches": log.mismatched,
            "tasks_unfinished": log.unfinished, "warnings": len(log.warnings), "warning_sample": log.warnings[:5],
            "tiles_rendered": rendered, "tiles_skipped": sum(t["skipped"] for t in log.tasks),
            "tiles_deleted": sum(t["deleted"] for t in log.tasks), "lowres_saves": sum(t["lowres"] for t in log.tasks),
            "tiles_rendered_per_s": round(rendered / span, 1),
            "regions_per_task_p50": spread([t["regions"] for t in log.tasks])["p50"],
            "cpu_total_s": round(cpu_total, 1), "rss_peak_mb": round(rss_peak / 2**20), "during_crawl": run_window,
            "crawl_and_drain": all_window, "mca_bytes_written": store.bytes_written,
            "mca_final_bytes": sum(v for v in latest_sizes(versions).values()),
            "rstate_bytes": tree_bytes(map_dir / "rstate"), "hires_bytes": tree_bytes(map_dir / "hires"),
            "lowres_bytes": tree_bytes(map_dir / "tiles"),
        })
        if a.settle and chunks:
            states = tile_states(map_dir)
            stamps = lambda c: store.regions[c[0] >> 5, c[1] >> 5].stamps.get((c[0] & 31) | ((c[1] & 31) << 5), 0)
            final = sum(1 for s, _ in states.values() if s.startswith("rendered"))
            summary["final_tiles_rendered"] = final
            summary["render_amplification"] = round(rendered / max(1, final), 3)
            summary["tile_states"] = count_states(states)
            summary.update(tile_latency(states, store.offered, stamps))
        if a.compare and free_bytes(Path("/dev/shm")) > 2 * summary["hires_bytes"] + MIN_FREE_SHM:
            clean, clean_dir = clean_render(a, world, a.threads)
            summary["clean"] = clean
            summary["compare"] = compare_maps(clean_dir / "web" / "maps" / "real", map_dir)
        elif a.compare:
            summary["compare"] = "skipped: not enough tmpfs for the clean render"
        return summary
    finally:
        fcntl.flock(lock, fcntl.LOCK_UN)
        lock.close()
        if not a.keep:
            shutil.rmtree(SHM / a.label, ignore_errors=True)
            shutil.rmtree(base, ignore_errors=True)
        record(a, summary)


def latest_sizes(versions) -> dict:
    return {v.region: v.size for v in versions}


def count_states(states: dict) -> dict:
    out: dict[str, int] = {}
    for s, _ in states.values():
        out[s] = out.get(s, 0) + 1
    return out


def record(a, summary: dict) -> None:
    with open(Path(a.out) / "results.jsonl", "a") as f:
        f.write(json.dumps(summary) + "\n")
    brief = {k: summary.get(k) for k in ("label", "describe", "offered_chunks_per_s", "regions_completed_per_s", "bounded",
                                          "backlog_q2_chunks", "backlog_q4_chunks", "render_amplification", "aborted")}
    print(json.dumps(brief), f"cores {summary.get('during_crawl', {}).get('cores_avg')}", flush=True)
    if a.compare:
        print(f"compare: {summary.get('compare')}", flush=True)


def sweep(a) -> None:
    """Rungs grow by 40% until one's backlog isn't bounded, then bisect once; the best gets the long, checked run."""
    def rung(rate: float, step: str, long: bool):
        args = argparse.Namespace(**vars(a))
        args.rate, args.label = rate, f"{a.label}-{step}"
        args.duration, args.settle, args.compare = (a.final, True, True) if long else (a.rung, False, False)
        return run(args)

    rate, good, bad, i = a.start, None, None, 0
    while bad is None and i < 8:
        s = rung(rate, f"r{i}", False)
        i += 1
        if s.get("bounded") and not s.get("aborted"):
            good, rate = rate, rate * 1.4
        else:
            bad = rate
    while good is None and bad > 50:
        bad /= 1.6
        s = rung(bad, f"d{i}", False)
        i += 1
        good = bad if s.get("bounded") else None
    if good is None:
        raise SystemExit("no bounded rate above 50 chunks/s")
    if bad is None or bad <= good:
        bad = good * 1.6
    mid = (good + bad) / 2
    s = rung(mid, f"b{i}", False)
    if s.get("bounded") and not s.get("aborted"):
        good = mid
    final = rung(min(good, a.final_cap), "final", True)
    if not final.get("bounded"):
        rung(good * 0.85, "final2", True)


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)

    def common(s: argparse.ArgumentParser) -> None:
        s.add_argument("label")
        s.add_argument("--pattern", choices=["spiral", "scatter"], required=True)
        s.add_argument("--threads", type=int, default=12)
        s.add_argument("--exe", type=Path, default=HOME / "ingest" / "bin" / "bluemap")
        s.add_argument("--out", default=str(HOME / "ingest"))
        s.add_argument("--large", action="store_true")
        s.add_argument("--bots", type=int)
        s.add_argument("--speed", type=float)
        s.add_argument("--keep", action="store_true")
        s.add_argument("--relight-unchanged", action="store_true", help="relit columns keep their timestamp")
        s.add_argument("--settle-limit", type=float, default=1800)
        s.add_argument("--perf", type=int, default=0, help="seconds of perf profile, from 0.7 of the crawl")

    r = sub.add_parser("run")
    common(r)
    r.add_argument("--rate", type=float, default=0.0)
    r.add_argument("--duration", type=float, default=600)
    r.add_argument("--settle", action="store_true")
    r.add_argument("--compare", action="store_true")
    s = sub.add_parser("sweep")
    common(s)
    s.add_argument("--start", type=float, required=True)
    s.add_argument("--rung", type=float, default=240)
    s.add_argument("--final", type=float, default=600)
    s.add_argument("--final-cap", type=float, default=3800, help="10 min above this doesn't fit in tmpfs (8.9 KB a chunk)")
    a = p.parse_args()
    Path(a.out).mkdir(parents=True, exist_ok=True)
    if a.cmd == "run":
        run(a)
    else:
        sweep(a)


if __name__ == "__main__":
    main()
