"""Tile load against a running BlueMap webserver, for RSS/allocation soaks of bm-web.

    python3 tools/web_load.py --url http://127.0.0.1:8137 --map real [--minutes M] [--threads N] [--range R]

Requests random hires (`tiles/0`) and lowres (`tiles/1..3`) tiles within ±R hires tiles of the origin from three
client kinds: packed (Accept BMQ3 + zstd, the unpacking script), legacy gzip (server-side unpack) and identity.
Prints requests/s and the status histogram per client kind.
"""
import argparse
import collections
import random
import threading
import time
import urllib.error
import urllib.request

CLIENTS = {
    "packed": ({"Accept": "application/vnd.bluemap.BMQ3, */*", "Accept-Encoding": "zstd, gzip"}, 6),
    "gzip": ({"Accept-Encoding": "gzip"}, 3),
    "identity": ({"Accept-Encoding": "identity"}, 1),
}


def split(n: int) -> str:
    s = str(abs(n))
    return ("-" if n < 0 else "") + "/".join(s)


def tile_path(rng: random.Random, map_id: str, r: int) -> str:
    lod = rng.choices([0, 1, 2, 3], weights=[8, 2, 1, 1])[0]
    span = r if lod == 0 else max(1, r >> (2 * (lod - 1)))
    x, z = rng.randint(-span, span - 1), rng.randint(-span, span - 1)
    ext = "prbm" if lod == 0 else "png"
    return f"maps/{map_id}/tiles/{lod}/x{split(x)}/z{split(z)}.{ext}"


def worker(base: str, map_id: str, r: int, end: float, seed: int, stats: dict, lock: threading.Lock) -> None:
    rng = random.Random(seed)
    names = list(CLIENTS)
    weights = [CLIENTS[n][1] for n in names]
    while time.monotonic() < end:
        kind = rng.choices(names, weights)[0]
        req = urllib.request.Request(f"{base}/{tile_path(rng, map_id, r)}", headers=CLIENTS[kind][0])
        try:
            with urllib.request.urlopen(req, timeout=30) as res:
                res.read()
                status = res.status
        except urllib.error.HTTPError as e:
            status = e.code
        except OSError as e:
            status = type(e).__name__
        with lock:
            stats[kind][status] += 1


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--url", required=True)
    ap.add_argument("--map", required=True)
    ap.add_argument("--minutes", type=float, default=10)
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--range", type=int, default=6)
    a = ap.parse_args()
    stats = {k: collections.Counter() for k in CLIENTS}
    lock = threading.Lock()
    end = time.monotonic() + a.minutes * 60
    threads = [threading.Thread(target=worker, args=(a.url, a.map, a.range, end, i, stats, lock)) for i in range(a.threads)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    total = sum(sum(c.values()) for c in stats.values())
    print(f"== web load: {total} requests, {total / (a.minutes * 60):.0f}/s")
    for kind, c in stats.items():
        print(f"  {kind}: " + ", ".join(f"{s}={n}" for s, n in sorted(c.items(), key=str)))


if __name__ == "__main__":
    main()
