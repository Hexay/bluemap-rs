"""Sync a checkout to the testbox and run commands there without hand-written ssh/scp/poll loops.

    py -3 tools/testbox.py sync <name> [--tree DIR]
    py -3 tools/testbox.py run  <name> [--sync] [--lock] [--job J] [--tail N] [--grep RE] [--detach] -- <command…>
    py -3 tools/testbox.py wait <name> [--job J] [--tail N] [--grep RE]

<name> is the remote checkout ~/bmrs-<name>. `sync` pushes the working tree as it is on disk (HEAD + uncommitted +
untracked, minus .gitignore) and hard-resets the remote checkout to it; ignored remote files (target/, work/) survive,
so builds stay incremental. `run` starts the command detached in that checkout (cargo on PATH; `--lock` holds
~/bench.lock for benchmarks), then blocks until it exits, prints the log tail and exits with its exit code. A dropped
connection only interrupts the wait: it reconnects, and `wait` re-attaches at any time (also after `--detach`).

Jobs outlive the 2-minute foreground limit: call `run`/`wait` with run_in_background and act on the notification.
"""
import argparse
import os
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

HOST = os.environ.get("BM_TESTBOX", "testbox")
SSH = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "-o", "ServerAliveInterval=30", "-o", "ServerAliveCountMax=4"]
SSH_DROPPED = 255
JOB_LOST = 3
MAX_RECONNECTS = 40


def remote(script: str) -> subprocess.CompletedProcess:
    # bytes, not text: text mode would send CRLF from Windows and bash would choke on the \r
    return subprocess.run([*SSH, HOST, "bash", "-s"], input=script.encode(), stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def remote_checked(script: str) -> str:
    done = remote("set -e\n" + script)
    if done.returncode != 0:
        sys.exit(f"testbox: remote step failed ({done.returncode}): {done.stderr.decode(errors='replace').strip()}")
    return done.stdout.decode(errors="replace")


def git(tree: Path, *args: str, env: dict | None = None) -> str:
    return subprocess.run(["git", "-C", str(tree), *args], check=True, capture_output=True, text=True, env=env).stdout.strip()


def snapshot_commit(tree: Path) -> str:
    """A commit of the working tree as it is on disk, made through a scratch index so the real one is untouched."""
    index = Path(git(tree, "rev-parse", "--path-format=absolute", "--git-path", "index"))
    with tempfile.TemporaryDirectory() as tmp:
        scratch = Path(tmp) / "index"
        shutil.copyfile(index, scratch)
        env = {**os.environ, "GIT_INDEX_FILE": str(scratch)}
        git(tree, "add", "-A", env=env)
        tree_id = git(tree, "write-tree", env=env)
    return git(tree, "commit-tree", tree_id, "-p", "HEAD", "-m", "testbox sync")


def sync(name: str, tree: Path) -> None:
    started = time.monotonic()
    commit = snapshot_commit(tree)
    remote_checked(f"git init -q ~/bmrs-{name}")
    push = subprocess.run(["git", "-C", str(tree), "push", "-q", "--force", f"{HOST}:bmrs-{name}", f"{commit}:refs/testbox/sync"],
                          capture_output=True, text=True)
    if push.returncode != 0:
        sys.exit(f"testbox: push failed: {push.stderr.strip()}")
    head = remote_checked(f"cd ~/bmrs-{name}\ngit reset -q --hard refs/testbox/sync\ngit log --oneline -1 HEAD~1")
    print(f"synced {tree} -> {HOST}:~/bmrs-{name} (on {head.strip()}) in {time.monotonic() - started:.0f}s")


def start(name: str, job: str, command: str, lock: bool) -> None:
    runner = f"flock ~/bench.lock bash $d/{job}.sh" if lock else f"bash $d/{job}.sh"
    remote_checked(f"""d=~/.testbox/{name}; mkdir -p $d
test -d ~/bmrs-{name}
cat > $d/{job}.sh <<'TESTBOX_EOF'
[ -f ~/.cargo/env ] && . ~/.cargo/env
cd ~/bmrs-{name}
{command}
TESTBOX_EOF
rm -f $d/{job}.exit
nohup setsid bash -c "{runner}; echo \\$? > $d/{job}.exit" > $d/{job}.log 2>&1 < /dev/null &
echo $! > $d/{job}.pid
""")


def wait(name: str, job: str, tail: int, grep: str | None) -> int:
    """Blocks until the job has exited, prints its log tail and returns its exit code."""
    show = f"grep -E -- {shell_quote(grep)} $d/{job}.log | tail -n {tail}" if grep else f"tail -n {tail} $d/{job}.log"
    script = f"""d=~/.testbox/{name}
[ -f $d/{job}.pid ] || {{ echo "no job '{job}' for {name}" >&2; exit {JOB_LOST}; }}
while [ ! -f $d/{job}.exit ]; do
  if ! kill -0 "$(cat $d/{job}.pid)" 2>/dev/null; then
    sleep 2
    [ -f $d/{job}.exit ] || {{ echo "job '{job}' died without an exit code (reboot or kill)" >&2; tail -n 20 $d/{job}.log >&2; exit {JOB_LOST}; }}
  fi
  sleep 2
done
echo "$(cat $d/{job}.exit) $(( $(stat -c %Y $d/{job}.exit) - $(stat -c %Y $d/{job}.pid) ))"
{show}
"""
    drops = 0
    while True:
        done = remote(script)
        if done.returncode == SSH_DROPPED and drops < MAX_RECONNECTS:
            drops += 1
            time.sleep(15)
            continue
        if done.returncode != 0:
            sys.stderr.write(done.stderr.decode(errors="replace"))
            return done.returncode
        header, _, log = done.stdout.decode(errors="replace").partition("\n")
        code, seconds = header.split()
        sys.stdout.write(log)
        print(f"-- {name}/{job}: exit {code} after {seconds}s" + (f" ({drops} reconnects)" if drops else ""))
        return int(code)


def shell_quote(text: str) -> str:
    return "'" + text.replace("'", "'\\''") + "'"


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = parser.add_subparsers(dest="action", required=True)

    def add(action: str) -> argparse.ArgumentParser:
        p = sub.add_parser(action)
        p.add_argument("name", help="remote checkout ~/bmrs-<name>")
        return p

    def add_output_flags(p: argparse.ArgumentParser) -> None:
        p.add_argument("--job", default="job", help="job name, for several jobs in one checkout")
        p.add_argument("--tail", type=int, default=40, help="log lines to print")
        p.add_argument("--grep", help="print only log lines matching this extended regex")

    tree_default = Path(__file__).resolve().parent.parent
    add("sync").add_argument("--tree", type=Path, default=tree_default)
    run = add("run")
    run.add_argument("--tree", type=Path, default=tree_default)
    run.add_argument("--sync", action="store_true", help="sync the tree first")
    run.add_argument("--lock", action="store_true", help="hold ~/bench.lock while the command runs")
    run.add_argument("--detach", action="store_true", help="start and return; collect with `wait`")
    add_output_flags(run)
    add_output_flags(add("wait"))
    # split by hand: argparse.REMAINDER would swallow the flags that follow <name>
    argv = sys.argv[1:]
    cut = argv.index("--") if "--" in argv else len(argv)
    args = parser.parse_args(argv[:cut])

    if args.action == "sync":
        sync(args.name, args.tree)
        return
    if args.action == "run":
        command = " ".join(argv[cut + 1:])
        if not command:
            parser.error("run needs -- <command>")
        if args.sync:
            sync(args.name, args.tree)
        start(args.name, args.job, command, args.lock)
        if args.detach:
            print(f"started {args.name}/{args.job}; collect with: py -3 tools/testbox.py wait {args.name} --job {args.job}")
            return
    sys.exit(wait(args.name, args.job, args.tail, args.grep))


if __name__ == "__main__":
    main()
