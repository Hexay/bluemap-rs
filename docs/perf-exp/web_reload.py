"""Page-load and reload-revalidation passes over one map view, as a browser would make them; prints one JSON line.

    python3 web_reload.py <host:port> <webroot> <map> --pid <server pid> [--full 5] [--reload 20] [--conns 6]

The view: index.html, both settings.json, textures.json, the 49 hires tiles nearest the map's median tile and the 9
nearest lowres tiles of each lod. A full pass sends no validators; a reload pass sends `Cache-Control: no-cache` plus
the `If-None-Match` / `If-Modified-Since` each URL's first reply carried (what a browser sends for a no-cache fetch of
a cached entry). Wire bytes are everything read from the socket: status line, headers, chunk framing, body.
"""
import argparse
import json
import os
import re
import socket
import statistics
import threading
import time
from collections import Counter

AE = "gzip, deflate, br, zstd"
TILE = re.compile(r"x(-?\d+)z(-?\d+)")


def view_urls(webroot: str, map_id: str) -> list[str]:
    tiles = os.path.join(webroot, "maps", map_id, "tiles")
    urls = ["index.html", "settings.json", f"maps/{map_id}/settings.json", f"maps/{map_id}/textures.json"]
    for lod in sorted(os.listdir(tiles), key=int):
        coords = []
        for dirpath, _, files in os.walk(os.path.join(tiles, lod)):
            for f in files:
                rel = os.path.relpath(os.path.join(dirpath, f), os.path.join(tiles, lod)).replace(os.sep, "")
                m = TILE.match(rel)
                if m:
                    coords.append((int(m[1]), int(m[2]), os.path.relpath(os.path.join(dirpath, f), webroot)))
        mx = statistics.median_low([c[0] for c in coords])
        mz = statistics.median_low([c[1] for c in coords])
        k = 49 if lod == "0" else 9
        near = sorted(coords, key=lambda c: (max(abs(c[0] - mx), abs(c[1] - mz)), c[0], c[1]))[:k]
        # hires URLs drop the stored .gz: the webapp asks for .prbm and negotiates the encoding
        urls += [c[2].replace(os.sep, "/").removesuffix(".gz") for c in near]
    return urls


class Conn:
    def __init__(self, addr: str):
        host, port = addr.rsplit(":", 1)
        self.addr = addr
        self.sock = socket.create_connection((host, int(port)))
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        self.f = self.sock.makefile("rb", buffering=256 * 1024)

    def get(self, url: str, extra: dict[str, str]) -> tuple[int, int, int, dict[str, str]]:
        """(status, body bytes, wire bytes, lower-cased headers)"""
        hdrs = "".join(f"{k}: {v}\r\n" for k, v in {"Accept-Encoding": AE, **extra}.items())
        self.sock.sendall(f"GET /{url} HTTP/1.1\r\nHost: {self.addr}\r\n{hdrs}\r\n".encode())
        line = self.f.readline()
        wire = len(line)
        status = int(line.split()[1])
        headers = {}
        while True:
            line = self.f.readline()
            wire += len(line)
            if line in (b"\r\n", b"\n", b""):
                break
            k, _, v = line.decode("latin-1").partition(":")
            headers[k.strip().lower()] = v.strip()
        body = 0
        if status in (204, 304):
            pass
        elif headers.get("transfer-encoding", "").lower() == "chunked":
            while True:
                line = self.f.readline()
                n = int(line.strip(), 16)
                chunk = self.f.read(n + 2)
                wire += len(line) + len(chunk)
                body += n
                if n == 0:
                    break
        else:
            n = int(headers.get("content-length", "0"))
            body = len(self.f.read(n))
            wire += body
        return status, body, wire, headers


def run_pass(addr: str, urls: list[str], conns: int, extra_for) -> dict:
    """Splits the view over `conns` keep-alive connections opened beforehand; times only the requests."""
    parts = [urls[i::conns] for i in range(conns)]
    sockets = [Conn(addr) for _ in parts]
    results: list[list] = [[] for _ in parts]

    def work(i: int) -> None:
        for u in parts[i]:
            results[i].append((u, *sockets[i].get(u, extra_for(u))))

    started = time.perf_counter()
    threads = [threading.Thread(target=work, args=(i,)) for i in range(len(parts))]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    wall = time.perf_counter() - started
    for s in sockets:
        s.sock.close()
    rows = [r for part in results for r in part]
    return {"wall_ms": wall * 1000, "rows": rows}


def cpu_ticks(pid: int) -> int:
    with open(f"/proc/{pid}/stat") as f:
        fields = f.read().rsplit(")", 1)[1].split()
    return int(fields[11]) + int(fields[12])


def summarize(passes: list[dict], ticks: int) -> dict:
    rows = [r for p in passes for r in p["rows"]]
    hires = [r for r in rows if r[0].endswith(".prbm")]
    return {
        "passes": len(passes),
        "requests_per_pass": len(passes[0]["rows"]),
        "wall_ms_median": round(statistics.median(p["wall_ms"] for p in passes), 2),
        "status": dict(Counter(str(r[1]) for r in passes[0]["rows"])),
        "wire_kb_per_pass": round(sum(r[3] for r in passes[0]["rows"]) / 1024, 1),
        "hires_wire_kb_per_tile": round(sum(r[3] for r in hires) / max(len(hires), 1) / 1024, 2),
        "cpu_ms_per_request": round(ticks * 1000 / os.sysconf("SC_CLK_TCK") / len(rows), 3),
    }


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("addr")
    ap.add_argument("webroot")
    ap.add_argument("map")
    ap.add_argument("--pid", type=int, required=True)
    ap.add_argument("--full", type=int, default=5)
    ap.add_argument("--reload", type=int, default=20)
    ap.add_argument("--conns", type=int, default=6)
    args = ap.parse_args()
    urls = view_urls(args.webroot, args.map)

    first = run_pass(args.addr, urls, args.conns, lambda u: {})
    validators = {}
    for u, _, _, _, h in first["rows"]:
        v = {}
        if "etag" in h:
            v["If-None-Match"] = h["etag"]
        if "last-modified" in h:
            v["If-Modified-Since"] = h["last-modified"]
        validators[u] = v
    have = Counter(" + ".join(sorted(v)) or "none" for v in validators.values())

    t0 = cpu_ticks(args.pid)
    full = [run_pass(args.addr, urls, args.conns, lambda u: {}) for _ in range(args.full)]
    t1 = cpu_ticks(args.pid)
    reload_extra = lambda u: {"Cache-Control": "no-cache", "Pragma": "no-cache", **validators[u]}
    reload = [run_pass(args.addr, urls, args.conns, reload_extra) for _ in range(args.reload)]
    t2 = cpu_ticks(args.pid)
    print(json.dumps({"validators": dict(have), "full": summarize(full, t1 - t0),
                      "reload": summarize(reload, t2 - t1)}))


if __name__ == "__main__":
    main()
