"""Acceptance for SQL storage on real MariaDB/MySQL/PostgreSQL servers (tools/dbs.py), against Java BlueMap 5.28:
  java->rs  Java renders into SQL; our `-r` over it renders nothing; both webservers serve identical responses.
  rs->java  we render into SQL; Java's webserver serves it identically to ours and to Java's golden tiles;
            Java's `-r` over it starts no work; both implementations create the same schema.
  optimized we render `format: optimized`, re-render nothing, serve, convert to compat (Java serves it) and back.

Usage: py -3 tools/accept_sql.py [--servers mariadb mysql postgres] [--fixture vanilla] [--no-build]
Output: work/accept/<fx>-sql-<server>-*/ (fresh each run). Exit 1 if any check fails; a hung or failed command
aborts only its step.
"""
import argparse
import gzip
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

import dbs
from accept import EXE, MC, ROOT, WORK, bluemap, check, prepare, run_bounded, set_conf, stop
import paths

# paths.WORK is this checkout's; accept.WORK also finds an enclosing one (worktrees)
JAVA, JAR = (WORK / p.relative_to(paths.WORK) for p in (paths.DEFAULT.bluemap_java, paths.DEFAULT.bluemap_jar))
TABLES = ["grid_storage_data", "item_storage_data", "grid_storage", "item_storage", "compression", "map"]


def configure(out: Path, s: dbs.Server, prefix: str, port: int, fmt: str = "compat") -> str:
    """Points every map of `out` at SQL storage `sql` (tables `prefix*`); returns the map id."""
    jar, cls = s.jdbc_driver()
    (out / "config" / "storages" / "sql.conf").write_text(
        f'storage-type: sql\nconnection-url: "{s.jdbc_url()}"\n'
        f'connection-properties: {{ user: "{s.user}", password: "{dbs.PASSWORD}" }}\n'
        f'max-connections: -1\ndriver-jar: "{jar.as_posix()}"\ndriver-class: "{cls}"\n'
        f'table-prefix: "{prefix}"\ncompression: gzip\nformat: {fmt}\n')
    set_conf(out / "config" / "webserver.conf", "port", str(port))
    conf = next((out / "config" / "maps").glob("*.conf"))
    set_conf(conf, "storage", '"sql"')
    return conf.stem


def drop(s: dbs.Server, prefix: str) -> None:
    for t in TABLES:
        dbs.query(s, f"DROP TABLE IF EXISTS {prefix}{t}")


def java_cmd(*flags: str) -> list[str]:
    return [str(JAVA), "-jar", str(JAR), "-c", "config", "-v", MC, *flags]


def java_server(cwd: Path) -> subprocess.Popen:
    # output discarded: an unread pipe would block the server once full
    return subprocess.Popen(java_cmd("-w"), cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def java_run(cwd: Path, *flags: str) -> str:
    start = time.monotonic()
    proc = run_bounded(java_cmd(*flags), cwd)
    out = proc.stdout + proc.stderr
    if proc.returncode:
        sys.exit(f"java bluemap failed in {cwd} ({proc.returncode}):\n{out[-3000:]}")
    summary = [line for line in out.splitlines() if re.search(r"Start updating|up-to-date|regions", line)]
    print(f"    java {' '.join(flags)}: {' | '.join(summary)} ({time.monotonic() - start:.1f}s)", flush=True)
    return out


def ours_server(cwd: Path) -> subprocess.Popen:
    return subprocess.Popen([str(EXE), "-c", "config", "-v", MC, "-w"], cwd=cwd, stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL)


def wait_http(port: int, timeout: float = 120) -> None:
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=1).read()
            return
        except urllib.error.HTTPError:
            return
        except OSError:
            time.sleep(0.2)
    sys.exit(f"no webserver on port {port}")


def get(port: int, path: str) -> tuple[int, str | None, bytes]:
    req = urllib.request.Request(f"http://127.0.0.1:{port}/{path}", headers={"Accept-Encoding": "gzip"})
    try:
        with urllib.request.urlopen(req, timeout=30) as res:
            return res.status, res.headers.get("Content-Encoding"), res.read()
    except urllib.error.HTTPError as e:
        return e.code, e.headers.get("Content-Encoding"), e.read()


def sample_paths(map_id: str, n: int = 40) -> list[str]:
    """settings/textures/live JSON, n hires and n lowres tiles of Java's golden webroot, and missing tiles."""
    golden = WORK / "bluemap" / FIXTURE / "web"
    hires = sorted((golden / "maps" / map_id / "tiles" / "0").rglob("*.prbm.gz"))
    lowres = sorted((golden / "maps" / map_id / "tiles").glob("[1-9]/**/*.png"))
    pick = lambda xs: [p.relative_to(golden).as_posix().removesuffix(".gz") for p in xs[:: max(1, len(xs) // n)]]
    base = f"maps/{map_id}"
    return [f"{base}/settings.json", f"{base}/textures.json", f"{base}/live/markers.json", f"{base}/live/players.json",
            *pick(hires), *pick(lowres), f"{base}/tiles/0/x9/9/z9/9/9.prbm", f"{base}/tiles/1/x9/9/z9/9/9.png"]


def decoded(res: tuple[int, str | None, bytes]) -> tuple[int, str | None, bytes]:
    return res[0], res[1], gzip.decompress(res[2]) if res[1] == "gzip" else res[2]


def served_equal(ports: tuple[int, int], paths: list[str], label: str) -> bool:
    """Stored tiles pass through byte-identical; JSON is compressed on the fly, so compares decoded (encoders differ)."""
    bad = 0
    for path in paths:
        a, b = get(ports[0], path), get(ports[1], path)
        if a != b and (path.endswith((".prbm", ".png")) or decoded(a) != decoded(b)):
            bad += 1
            if bad <= 5:
                print(f"    {label} differs: {path}: {a[:2]} {len(a[2])} B vs {b[:2]} {len(b[2])} B", flush=True)
    print(f"    {label}: {len(paths) - bad}/{len(paths)} responses identical", flush=True)
    return bad == 0


def served_golden(port: int, map_id: str) -> bool:
    """Every hires tile served on `port` decodes to Java's golden tile."""
    golden = WORK / "bluemap" / FIXTURE / "web"
    tiles = sorted((golden / "maps" / map_id / "tiles" / "0").rglob("*.prbm.gz"))
    for tile in tiles:
        status, enc, body = get(port, tile.relative_to(golden).as_posix().removesuffix(".gz"))
        if status != 200 or (gzip.decompress(body) if enc == "gzip" else body) != gzip.decompress(tile.read_bytes()):
            print(f"    tile differs from golden: {tile}", flush=True)
            return False
    print(f"    {len(tiles)} hires tiles equal Java's golden render", flush=True)
    return bool(tiles)


def schema(s: dbs.Server, prefix: str) -> list[str]:
    """Normalised DDL of the six tables (prefix replaced, auto-increment counters dropped)."""
    if s.name == "postgres":
        rows = dbs.query(s, f"""
            SELECT table_name, column_name, data_type, is_nullable, coalesce(column_default, '') FROM information_schema.columns
             WHERE table_schema = current_schema() AND table_name LIKE '{prefix}%' ORDER BY table_name, ordinal_position""")
        rows += dbs.query(s, f"""
            SELECT conrelid::regclass::text, conname, pg_get_constraintdef(oid) FROM pg_constraint
             WHERE conrelid::regclass::text LIKE '{prefix}%' ORDER BY 1, 2""")
        rows += dbs.query(s, f"SELECT tablename, indexname, indexdef FROM pg_indexes WHERE tablename LIKE '{prefix}%' ORDER BY 1, 2")
        lines = ["\t".join(r) for r in rows]
    else:
        lines = []
        for t in TABLES:
            ddl = dbs.query(s, f"SHOW CREATE TABLE {prefix}{t}")[0][1].replace("\\n", "\n")
            lines += re.sub(r" AUTO_INCREMENT=\d+", "", ddl).splitlines()
    return [line.replace(prefix, "<prefix>") for line in lines]


def java_to_rs(s: dbs.Server, failures: list[str], ports: tuple[int, int]) -> None:
    print(f"{s.name}: Java renders, we read", flush=True)
    prefix = "bmjava_"
    drop(s, prefix)
    jdir, rdir = prepare(FIXTURE, f"{FIXTURE}-sql-{s.name}-java"), prepare(FIXTURE, f"{FIXTURE}-sql-{s.name}-rs")
    map_id = configure(jdir, s, prefix, ports[0])
    configure(rdir, s, prefix, ports[1])
    java_run(jdir, "-r")
    again = bluemap(rdir, "-r")
    check(bool(again) and all(r == 0 for r, _ in again.values()), f"{s.name}: our -r over Java's SQL renders nothing", failures)
    serve_both(jdir, rdir, ports, map_id, f"{s.name}: Java-rendered SQL", failures)


def serve_both(jdir: Path, rdir: Path, ports: tuple[int, int], map_id: str, label: str, failures: list[str]) -> None:
    jserver, rserver = java_server(jdir), ours_server(rdir)
    try:
        wait_http(ports[0])
        wait_http(ports[1])
        check(served_equal(ports, sample_paths(map_id), label), f"{label}: our webserver == Java's", failures)
        check(served_golden(ports[0], map_id), f"{label}: Java's webserver serves golden tiles", failures)
    finally:
        stop(jserver)
        stop(rserver)


def rs_to_java(s: dbs.Server, failures: list[str], ports: tuple[int, int]) -> None:
    print(f"{s.name}: we render, Java reads", flush=True)
    prefix = "bmrs_"
    drop(s, prefix)
    jdir, rdir = prepare(FIXTURE, f"{FIXTURE}-sql-{s.name}-java2"), prepare(FIXTURE, f"{FIXTURE}-sql-{s.name}-rs2")
    map_id = configure(jdir, s, prefix, ports[0])
    configure(rdir, s, prefix, ports[1])
    bluemap(rdir, "-r")
    serve_both(jdir, rdir, ports, map_id, f"{s.name}: our SQL", failures)
    before = fingerprint(s, prefix)
    java_run(jdir, "-r")
    check(fingerprint(s, prefix) == before, f"{s.name}: Java -r over our SQL rewrites no tile", failures)
    same = schema(s, "bmjava_") == schema(s, prefix)
    if not same:
        a, b = schema(s, "bmjava_"), schema(s, prefix)
        print("    java only: " + "\n      ".join(x for x in a if x not in b))
        print("    ours only: " + "\n      ".join(x for x in b if x not in a))
    check(same, f"{s.name}: schema identical to Java's", failures)


def optimized(s: dbs.Server, failures: list[str], ports: tuple[int, int]) -> None:
    print(f"{s.name}: optimized format", flush=True)
    prefix = "bmopt_"
    drop(s, prefix)
    jdir, rdir = prepare(FIXTURE, f"{FIXTURE}-sql-{s.name}-java3"), prepare(FIXTURE, f"{FIXTURE}-sql-{s.name}-opt")
    map_id = configure(jdir, s, prefix, ports[0])
    configure(rdir, s, prefix, ports[1], "optimized")
    bluemap(rdir, "-r")
    again = bluemap(rdir, "-r")
    check(bool(again) and all(r == 0 for r, _ in again.values()), f"{s.name}: optimized second run renders nothing", failures)
    rows = dbs.query(s, f"SELECT COUNT(*) FROM {prefix}grid_storage_data d JOIN {prefix}grid_storage g ON d.storage = g.id "
                        f"WHERE g.{quote(s, 'key')} = 'bluemap:hires'")
    check(rows == [["0"]], f"{s.name}: optimized stores no upstream hires rows", failures)
    server = ours_server(rdir)
    try:
        wait_http(ports[1])
        check(served_golden(ports[1], map_id), f"{s.name}: optimized served tiles equal golden", failures)
    finally:
        stop(server)
    bluemap(rdir, "--convert-storage", "sql", "--to", "compat")
    configure(rdir, s, prefix, ports[1], "compat")
    serve_both(jdir, rdir, ports, map_id, f"{s.name}: optimized->compat", failures)
    configure(rdir, s, prefix, ports[1], "optimized")
    bluemap(rdir, "--convert-storage", "sql", "--to", "optimized")
    again = bluemap(rdir, "-r")
    check(bool(again) and all(r == 0 for r, _ in again.values()), f"{s.name}: compat->optimized renders nothing", failures)


def fingerprint(s: dbs.Server, prefix: str) -> list[list[str]]:
    """Row count and content hash of hires and lowres cells per storage key (a Java re-render would re-encode tiles
    with its own encoders; render-state cells are rewritten on every save, so they don't count)."""
    hash_ = "md5(string_agg(md5(d.data), '' ORDER BY d.x, d.z))" if s.name == "postgres" else "SUM(CRC32(d.data))"
    return dbs.query(s, f"SELECT g.{quote(s, 'key')}, COUNT(*), {hash_} FROM {prefix}grid_storage_data d "
                        f"JOIN {prefix}grid_storage g ON d.storage = g.id WHERE g.{quote(s, 'key')} LIKE 'bluemap:%res%' "
                        f"GROUP BY g.{quote(s, 'key')} ORDER BY 1")


def quote(s: dbs.Server, col: str) -> str:
    return f'"{col}"' if s.name == "postgres" else f"`{col}`"


FIXTURE = "vanilla"


def main() -> None:
    global FIXTURE
    ap = argparse.ArgumentParser()
    ap.add_argument("--servers", nargs="*", default=["mariadb", "mysql", "postgres"])
    ap.add_argument("--fixture", default=FIXTURE)
    ap.add_argument("--no-build", action="store_true")
    args = ap.parse_args()
    FIXTURE = args.fixture
    if not args.no_build:
        subprocess.run(["cargo", "build", "--release", "-p", "bm-cli"], cwd=ROOT, check=True)
    failures: list[str] = []
    for i, name in enumerate(args.servers):
        s = dbs.SERVERS[name]
        dbs.start(s)
        ports = (18200 + 2 * i, 18201 + 2 * i)
        for step in (java_to_rs, rs_to_java, optimized):
            # a hang or crash aborts this step only: the other steps and servers still run and report
            try:
                step(s, failures, ports)
            except SystemExit as e:
                print(f"  ABORT {name}: {step.__name__}: {e.code}", flush=True)
                failures.append(f"{name}: {step.__name__} aborted: {str(e.code).splitlines()[0]}")
    print(f"\n{len(failures)} failed" + "".join(f"\n  {f}" for f in failures))
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
