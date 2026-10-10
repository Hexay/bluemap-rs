"""usage: web_bench.py <webroot> [--secs 8] [--conns 32] [--only name,…] [--log file] [--sse N] [--attach]
                    [--server-arg ARG]… [--urls-from <compat webroot>]

Starts examples/serve_bench on <webroot>, runs examples/load_bench scenarios against it, and prints one table row
per scenario: throughput, latency percentiles, server CPU ms/request, allocations/request, working set.
Build first: cargo build -p bm-web --profile profiling --examples
"""
import argparse, json, random, subprocess, sys, tempfile, time
from pathlib import Path

import psutil

ROOT = Path(__file__).resolve().parents[2]
EXE = ROOT / "target" / "profiling" / "examples"
BROWSER_AE = "Accept-Encoding: gzip, deflate, br, zstd"


def tile_urls(map_dir: Path, lod: int, ext: str):
    base = map_dir / "tiles" / str(lod)
    out = []
    for p in base.rglob("*" + ext + (".gz" if ext == ".prbm" else "")):
        rel = p.relative_to(map_dir).as_posix()
        out.append(f"maps/{map_dir.name}/" + rel.removesuffix(".gz"))
    return sorted(out)


def view_urls(map_id: str, cx: int, cz: int):
    """What one webapp view requests: 49 hires (r=3) + 81 lowres per LOD × 3 (docs/11 §3)."""
    hires = [f"maps/{map_id}/tiles/0/{split('x', cx + dx)}{split('z', cz + dz)}.prbm"
             for dx in range(-3, 4) for dz in range(-3, 4)]
    low = [f"maps/{map_id}/tiles/{lod}/x{dx}/z{dz}.png" for lod in (1, 2, 3)
           for dx in range(-4, 5) for dz in range(-4, 5)]
    return hires, low


def split(axis: str, v: int):
    """bm_format digit-split path: x-12 → x-1/2/ (last digit is the file name, joined by the caller)."""
    s = str(v)
    sign, digits = ("-", s[1:]) if s.startswith("-") else ("", s)
    head = "/".join(digits)
    return f"{axis}{sign}{head}" + ("/" if axis == "x" else "")


def scenarios(webroot: Path):
    maps = sorted(p for p in (webroot / "maps").iterdir() if p.is_dir())
    m = maps[0]
    hires = tile_urls(m, 0, ".prbm")
    lowres = tile_urls(m, 1, ".png") + tile_urls(m, 2, ".png") + tile_urls(m, 3, ".png")
    js = next((webroot / "assets").glob("index-*.js")).name
    css = next((webroot / "assets").glob("index-*.css")).name
    small = ["index.html", "settings.json", "lang/settings.conf", "lang/en.conf", f"assets/{css}"]
    vh, vl = view_urls(m.name, 0, 0)
    page = (["index.html", f"assets/{js}", f"assets/{css}", "settings.json", "lang/settings.conf", "lang/en.conf",
             f"maps/{m.name}/settings.json", f"maps/{m.name}/textures.json"] + vh + vl)
    missing = [f"maps/{m.name}/tiles/0/{split('x', 900 + i)}{split('z', 900)}.prbm" for i in range(200)]
    ims = "If-Modified-Since: Fri, 1 Jan 2100 00:00:00 GMT"
    return m.name, [
        ("hires_gzip", hires, [BROWSER_AE]),
        ("hires_gzip_only", hires, ["Accept-Encoding: gzip"]),
        ("hires_identity", hires, []),
        ("lowres_png", lowres, [BROWSER_AE]),
        ("missing_204", missing, [BROWSER_AE]),
        ("floor_404_no_io", [f"maps/{m.name}/nothing{i}" for i in range(50)], [BROWSER_AE]),
        ("map_settings_gzip", [f"maps/{m.name}/settings.json"], [BROWSER_AE]),
        ("textures_gzip", [f"maps/{m.name}/textures.json"], [BROWSER_AE]),
        ("textures_identity", [f"maps/{m.name}/textures.json"], []),
        ("static_js_1.2MB", [f"assets/{js}"], [BROWSER_AE]),
        ("static_small", small, [BROWSER_AE]),
        ("static_304", small, [BROWSER_AE, ims]),
        ("page_view_mix", page, [BROWSER_AE]),
        # a reload: every map-data URL of the view revalidated with the ETag it was served with (if any)
        ("reload_map_data", page[6:], [BROWSER_AE], ["--revalidate"]),
        ("reload_hires", vh, [BROWSER_AE], ["--revalidate"]),
        ("live_players", [f"maps/{m.name}/live/players.json"], [BROWSER_AE]),
    ]


class Server:
    def __init__(self, webroot, port, log, extra):
        cmd = [str(EXE / "serve_bench.exe"), str(webroot), "--port", str(port), "--live"] + extra
        if log:
            cmd += ["--log", log]
        self.proc = subprocess.Popen(cmd, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
        self.ps = None
        for _ in range(200):
            time.sleep(0.05)
            procs = [psutil.Process(self.proc.pid)] + psutil.Process(self.proc.pid).children(recursive=True)
            self.ps = next((p for p in procs if p.name().lower() == "serve_bench.exe"), None)
            if self.ps and any(c.status == "LISTEN" for c in self.ps.net_connections()):
                break
        time.sleep(0.3)

    def stats(self):
        self.proc.stdin.write("stats\n")
        return json.loads(self.proc.stdout.readline())

    def cpu(self):
        return sum(self.cpu_split())

    def cpu_split(self):
        t = self.ps.cpu_times()
        return t.user, t.system

    def mem(self):
        mi = self.ps.memory_info()
        return mi.wset, mi.peak_wset

    def stop(self):
        try:
            self.proc.stdin.close()
            self.ps.terminate()
        except (OSError, psutil.Error):
            pass
        self.proc.wait(30)


class Attached:
    """A server someone else started (e.g. under an elevated profiler): load numbers only."""

    class _Ps:
        pid = "?"

    ps = _Ps()

    def stats(self):
        return {"allocs": 0, "alloc_bytes": 0, "live_bytes": 0, "peak_live_bytes": 0}

    def cpu(self):
        return 0.0

    def cpu_split(self):
        return 0.0, 0.0

    def mem(self):
        return 0, 0

    def stop(self):
        pass


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("webroot")
    ap.add_argument("--secs", type=int, default=8)
    ap.add_argument("--conns", type=int, default=32)
    ap.add_argument("--only", default="")
    ap.add_argument("--port", type=int, default=8123)
    ap.add_argument("--log", default="")
    ap.add_argument("--sse", type=int, default=1000)
    ap.add_argument("--attach", action="store_true", help="drive an already running server on --port")
    ap.add_argument("--server-arg", action="append", default=[], help="extra serve_bench argument, e.g. --etags")
    ap.add_argument("--urls-from", help="compat webroot to list tile URLs from (an optimized one has no tile files)")
    a = ap.parse_args()
    webroot = Path(a.webroot)
    map_id, scen = scenarios(Path(a.urls_from or webroot))
    only = set(filter(None, a.only.split(",")))
    srv = Attached() if a.attach else Server(webroot, a.port, a.log, a.server_arg)
    addr = f"127.0.0.1:{a.port}"
    tmp = Path(tempfile.mkdtemp())
    print(f"server pid {srv.ps.pid}; idle wset {srv.mem()[0] / 1e6:.1f} MB; conns {a.conns}; {a.secs}s each")
    hdr = "| scenario | req/s | MB/s | p50 µs | p99 µs | max µs | CPU ms/req | allocs/req | KB alloc/req | wset MB | peak wset MB | 2xx/3xx/4xx/5xx/err |"
    print(hdr)
    print("|" + "---|" * (hdr.count("|") - 1))
    for name, urls, headers, *extra in scen:
        if only and name not in only:
            continue
        f = tmp / f"{name}.txt"
        random.Random(1).shuffle(urls)
        f.write_text("\n".join(urls))
        cmd = [str(EXE / "load_bench.exe"), addr, str(f), "--conns", str(a.conns), "--secs", str(a.secs)]
        cmd += extra[0] if extra else []
        for h in headers:
            cmd += ["--header", h]
        srv.stats()
        u0, k0 = srv.cpu_split()
        r = json.loads(subprocess.run(cmd, capture_output=True, text=True, check=True).stdout)
        u1, k1 = srv.cpu_split()
        cpu, kern = (u1 - u0) + (k1 - k0), k1 - k0
        s = srv.stats()
        n = max(r["requests"], 1)
        ws, peak = srv.mem()
        print(f"| {name} | {r['rps']:.0f} | {r['mb_per_s']:.1f} | {r['p50_us']} | {r['p99_us']} | {r['max_us']} | "
              f"{cpu * 1000 / n:.3f} ({kern / max(cpu, 1e-9) * 100:.0f}% kernel) | {s['allocs'] / n:.1f} | {s['alloc_bytes'] / n / 1024:.1f} | {ws / 1e6:.1f} | "
              f"{peak / 1e6:.1f} | {r['s2xx']}/{r['s3xx']}/{r['s4xx']}/{r['s5xx']}/{r['errors']} |", flush=True)
    if a.sse and (not only or "sse" in only):
        f = tmp / "sse.txt"
        f.write_text(f"maps/{map_id}/live/sse")
        srv.stats()
        c0 = srv.cpu()
        r = json.loads(subprocess.run([str(EXE / "load_bench.exe"), addr, str(f), "--sse", str(a.sse), "--secs",
                                       str(a.secs)], capture_output=True, text=True, check=True).stdout)
        s = srv.stats()
        ws, peak = srv.mem()
        print(f"\nsse: {a.sse} clients {a.secs}s: {r['bytes'] / a.secs / 1e3:.0f} KB/s total, server CPU "
              f"{(srv.cpu() - c0) * 1000 / a.secs:.1f} ms/s, allocs {s['allocs']}, peak live heap "
              f"{s['peak_live_bytes'] / 1e6:.1f} MB, wset {ws / 1e6:.1f} MB")
    srv.stop()


if __name__ == "__main__":
    sys.exit(main())
