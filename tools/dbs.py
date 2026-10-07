"""Portable MariaDB / MySQL / PostgreSQL servers for SQL storage tests (Windows, no install, no Docker).

Usage: py -3 tools/dbs.py start|stop|status [server ...] | jdbc   (default servers: mariadb postgres)
       servers: mariadb, mysql, postgres
Downloads the official Windows ZIPs and JDBC drivers into work/db/, initialises a data dir under work/db/data/<server>
(database `bluemap`, root/postgres password `bluemap`) and binds 127.0.0.1 on a non-default port. `start` prints the
sqlx URL (BM_TEST_*_URL) and the JDBC URL for Java BlueMap's sql.conf; `jdbc` fetches and lists the driver jars.
"""
import os
import shutil
import socket
import subprocess
import sys
import time
import urllib.request
import zipfile
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# work/ is git-ignored, so a worktree finds it in an enclosing checkout
WORK = next((p / "work" for p in [ROOT, *ROOT.parents] if (p / "work" / "downloads").exists()), ROOT / "work")
DB = WORK / "db"
PASSWORD = "bluemap"
HOST = "127.0.0.1"
DETACHED = 0x00000008 | 0x00000200  # DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP

MAVEN = "https://repo1.maven.org/maven2"
JDBC = {
    "mariadb": (f"{MAVEN}/org/mariadb/jdbc/mariadb-java-client/3.5.10/mariadb-java-client-3.5.10.jar", "org.mariadb.jdbc.Driver"),
    "postgres": (f"{MAVEN}/org/postgresql/postgresql/42.7.13/postgresql-42.7.13.jar", "org.postgresql.Driver"),
    "mysql": (f"{MAVEN}/com/mysql/mysql-connector-j/9.7.0/mysql-connector-j-9.7.0.jar", "com.mysql.cj.jdbc.Driver"),
}


@dataclass(frozen=True)
class Server:
    name: str
    url: str
    port: int
    user: str

    @property
    def archive(self) -> Path:
        return DB / "downloads" / self.url.rsplit("/", 1)[1]

    @property
    def home(self) -> Path:
        return DB / self.archive.stem

    @property
    def data(self) -> Path:
        return DB / "data" / self.name

    @property
    def log(self) -> Path:
        return DB / "data" / f"{self.name}.log"

    def bin(self, exe: str) -> Path:
        return next(self.home.glob(f"*/bin/{exe}.exe"))

    @property
    def scheme(self) -> str:
        return "postgres" if self.name == "postgres" else "mysql"

    def sqlx_url(self) -> str:
        return f"{self.scheme}://{self.user}:{PASSWORD}@{HOST}:{self.port}/bluemap"

    def jdbc_url(self) -> str:
        if self.name == "postgres":
            return f"jdbc:postgresql://{HOST}:{self.port}/bluemap"
        if self.name == "mysql":
            return f"jdbc:mysql://{HOST}:{self.port}/bluemap?allowPublicKeyRetrieval=true&useSSL=false"
        return f"jdbc:mariadb://{HOST}:{self.port}/bluemap"

    def jdbc_driver(self) -> tuple[Path, str]:
        """Driver jar (downloaded) and class for Java BlueMap's `driver-jar`/`driver-class`."""
        url, cls = JDBC[self.name]
        jar = DB / "jdbc" / url.rsplit("/", 1)[1]
        download(url, jar)
        return jar, cls


SERVERS = {
    s.name: s
    for s in [
        Server("mariadb", "https://archive.mariadb.org/mariadb-11.8.9/winx64-packages/mariadb-11.8.9-winx64.zip", 3407, "root"),
        Server("mysql", "https://cdn.mysql.com/Downloads/MySQL-8.4/mysql-8.4.9-winx64.zip", 3408, "root"),
        Server("postgres", "https://get.enterprisedb.com/postgresql/postgresql-18.4-1-windows-x64-binaries.zip", 5433, "postgres"),
    ]
}


def download(url: str, dest: Path) -> None:
    if dest.exists():
        return
    dest.parent.mkdir(parents=True, exist_ok=True)
    print(f"fetch   {dest.name}", flush=True)
    tmp = dest.with_suffix(dest.suffix + ".part")
    req = urllib.request.Request(url, headers={"User-Agent": "bluemap-rs-dbs"})
    with urllib.request.urlopen(req) as r, open(tmp, "wb") as f:
        shutil.copyfileobj(r, f, 1 << 20)
    tmp.replace(dest)


def install(s: Server) -> None:
    download(s.url, s.archive)
    if not s.home.exists():
        print(f"extract {s.archive.name}", flush=True)
        tmp = s.home.with_name(s.home.name + ".part")
        shutil.rmtree(tmp, ignore_errors=True)
        with zipfile.ZipFile(s.archive) as z:
            z.extractall(tmp)
        tmp.replace(s.home)


def up(s: Server) -> bool:
    try:
        with socket.create_connection((HOST, s.port), timeout=0.5):
            return True
    except OSError:
        return False


def wait_up(s: Server, timeout: float = 60) -> None:
    end = time.monotonic() + timeout
    while not up(s):
        if time.monotonic() > end:
            sys.exit(f"{s.name} did not come up; see {s.log}")
        time.sleep(0.2)


def run(*cmd, **kw) -> None:
    subprocess.run([str(c) for c in cmd], check=True, **kw)


def spawn(s: Server, *cmd) -> None:
    s.log.parent.mkdir(parents=True, exist_ok=True)
    with open(s.log, "ab") as log:
        subprocess.Popen([str(c) for c in cmd], stdout=log, stderr=log, stdin=subprocess.DEVNULL, creationflags=DETACHED)


def mysql_client(s: Server, sql: str, password: str | None = PASSWORD) -> None:
    exe = "mariadb" if s.name == "mariadb" else "mysql"
    auth = [f"-p{password}"] if password else []
    run(s.bin(exe), "-uroot", *auth, "-h", HOST, f"-P{s.port}", "-e", sql)


def query(s: Server, sql: str) -> list[list[str]]:
    """Rows of `sql` in database `bluemap`, tab-separated fields (newlines inside values are escaped)."""
    if s.name == "postgres":
        cmd = [s.bin("psql"), "-h", HOST, "-p", s.port, "-U", "postgres", "-d", "bluemap", "-At", "-F", "\t",
               "-v", "ON_ERROR_STOP=1", "-c", sql]
    else:
        exe = "mariadb" if s.name == "mariadb" else "mysql"
        cmd = [s.bin(exe), "-uroot", f"-p{PASSWORD}", "-h", HOST, f"-P{s.port}", "-D", "bluemap", "-N", "-B", "-e", sql]
    out = subprocess.run([str(c) for c in cmd], check=True, capture_output=True, text=True, env=pg_env()).stdout
    return [line.split("\t") for line in out.splitlines() if line]


def start(s: Server) -> None:
    if up(s):
        return
    install(s)
    fresh = not s.data.exists()
    s.data.parent.mkdir(parents=True, exist_ok=True)
    if s.name == "postgres":
        if fresh:
            pw = DB / "data" / "pgpass.tmp"
            pw.parent.mkdir(parents=True, exist_ok=True)
            pw.write_text(PASSWORD)
            run(s.bin("initdb"), "-D", s.data, "-U", "postgres", "-E", "UTF8", "--auth=scram-sha-256", f"--pwfile={pw}",
                stdout=subprocess.DEVNULL)
            pw.unlink()
        # MariaDB and MySQL generate TLS certificates themselves; PostgreSQL needs one for `sslmode=require`
        tls = ""
        if not (s.data / "server.crt").exists() and shutil.which("openssl"):
            run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "3650", "-subj", f"/CN={HOST}",
                "-keyout", s.data / "server.key", "-out", s.data / "server.crt", stderr=subprocess.DEVNULL)
        if (s.data / "server.crt").exists():
            tls = " -c ssl=on"
        # the server inherits pg_ctl's handles: any pipe would stay open until it stops
        run(s.bin("pg_ctl"), "-D", s.data, "-l", s.log, "-o", f"-p {s.port} -h {HOST}{tls}", "start",
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        wait_up(s)
        if fresh:
            run(s.bin("createdb"), "-h", HOST, "-p", s.port, "-U", "postgres", "bluemap", env=pg_env())
        return
    if s.name == "mariadb":
        if fresh:
            run(s.bin("mariadb-install-db"), f"--datadir={s.data}", f"--password={PASSWORD}", stdout=subprocess.DEVNULL)
        spawn(s, s.bin("mariadbd"), f"--datadir={s.data}", f"--port={s.port}", f"--bind-address={HOST}", "--console")
    else:
        if fresh:
            run(s.bin("mysqld"), "--initialize-insecure", f"--datadir={s.data}", stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL)
        spawn(s, s.bin("mysqld"), f"--datadir={s.data}", f"--port={s.port}", f"--bind-address={HOST}", "--mysqlx=OFF",
              "--console")
    wait_up(s)
    if fresh and s.name == "mysql":
        mysql_client(s, f"ALTER USER 'root'@'localhost' IDENTIFIED BY '{PASSWORD}'", password=None)
    if fresh:
        mysql_client(s, "CREATE DATABASE IF NOT EXISTS bluemap")


def pg_env() -> dict:
    return {**os.environ, "PGPASSWORD": PASSWORD}


def stop(s: Server) -> None:
    if not up(s):
        return
    if s.name == "postgres":
        run(s.bin("pg_ctl"), "-D", s.data, "stop", "-m", "fast", stdout=subprocess.DEVNULL)
    else:
        exe = "mariadb-admin" if s.name == "mariadb" else "mysqladmin"
        run(s.bin(exe), "-uroot", f"-p{PASSWORD}", "-h", HOST, f"-P{s.port}", "shutdown")
    end = time.monotonic() + 30
    while up(s) and time.monotonic() < end:
        time.sleep(0.2)


def jdbc() -> None:
    for s in SERVERS.values():
        jar, cls = s.jdbc_driver()
        print(f"{s.name:9} driver-jar: {jar.as_posix()}  driver-class: {cls}")


def status(s: Server) -> None:
    state = "up" if up(s) else "down"
    print(f"{s.name:9} {state:5} {s.sqlx_url()}  {s.jdbc_url()} (user {s.user}, password {PASSWORD})")


def main() -> None:
    if len(sys.argv) < 2 or sys.argv[1] not in ("start", "stop", "status", "jdbc"):
        sys.exit(__doc__)
    cmd, names = sys.argv[1], sys.argv[2:] or ["mariadb", "postgres"]
    if cmd == "jdbc":
        return jdbc()
    for name in names:
        if name not in SERVERS:
            sys.exit(f"unknown server {name}; one of {', '.join(SERVERS)}")
        s = SERVERS[name]
        if cmd == "start":
            start(s)
        elif cmd == "stop":
            stop(s)
        status(s)


if __name__ == "__main__":
    main()
