#!/bin/bash
# usage: devbox_web_distinct.sh <checkout> <java compat fixture> <optimized fixture> <java> <bluemap cli jar> [runs=3] [secs=5]
# Hires tiles nobody requested recently: Java's webserver and ours on the same compat files, ours on optimized
# storage of the same map (opt-in client that unpacks; and a client without the script). Same client and per-row
# method as devbox_web_java_vs_rs.sh; tiles of the 1,000-tile warmup set are never measured. warm: closed loop for
# SECS (these paths have no per-tile cache, so wrapping the list only re-reads page cache), except the no-script
# rows, which take each tile once (`--once`: the transcode cache would serve repeats). cold: page cache of the
# measured webroot dropped per file (posix_fadvise) after the warmup, then each tile once.
# Run under ~/bench.lock. Results: docs/results/devbox-web-java-vs-rs-2026-10-28.md
set -e
CO=$1; SRCJ=$2; SRCO=$3; JAVA=$4; JAR=$5; RUNS=${6:-3}; SECS=${7:-5}
EXE=$CO/target/profiling/examples
BIN=$CO/target/release/bluemap
OUT=${OUT:-$HOME/jvr-distinct}
PORT=8100
mkdir -p "$OUT"; rm -rf "$OUT/j" "$OUT/o"; mkdir -p "$OUT/j/data" "$OUT/o/data"
cp -a "$SRCJ/config" "$OUT/j/"; cp -al "$SRCJ/web" "$OUT/j/"
cp -a "$SRCO/config" "$OUT/o/"; cp -al "$SRCO/web" "$OUT/o/"
ROWS=$OUT/rows.jsonl; : > "$ROWS"
ss -ltn | grep -q ":$PORT " && { echo "port $PORT busy"; exit 1; }

MAP=$(ls "$OUT/j/web/maps" | head -1)
ALL=$OUT/hires.txt; WARM=$OUT/warmup.txt; B=$OUT/measured.txt; B1=$OUT/measured1500.txt
(cd "$OUT/j/web" && find "maps/$MAP/tiles/0" -name '*.prbm.gz' | sed 's/\.gz$//' | sort | shuf --random-source=<(yes)) > "$ALL"
head -1000 "$ALL" > "$WARM"; tail -n +1001 "$ALL" > "$B"; head -1500 "$B" > "$B1"
echo "$(wc -l < "$ALL") hires tiles of $MAP: $(wc -l < "$WARM") warmup, $(wc -l < "$B") measured"
TICK=$(getconf CLK_TCK)
cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat"; }
"$JAVA" -version 2>&1 | head -1
"$BIN" --version

root_of() { case $1 in java|rs) echo "$OUT/j" ;; *) echo "$OUT/o" ;; esac; }
warm_cache() { find "$(root_of "$1")/web/maps" -type f -exec cat {} + > /dev/null; }
evict() {
  python3 - "$(root_of "$1")/web/maps" <<'EOF'
import os, subprocess, sys
files = [os.path.join(d, f) for d, _, fs in os.walk(sys.argv[1]) for f in fs]
for p in files:
    fd = os.open(p, os.O_RDONLY)
    try:
        os.posix_fadvise(fd, 0, 0, os.POSIX_FADV_DONTNEED)
    finally:
        os.close(fd)
out = [w for i in range(0, len(files), 2000) for w in subprocess.run(
    ["fincore", "-b", "-n", "-o", "RES,SIZE", *files[i:i + 2000]], capture_output=True, text=True).stdout.split()]
print(f"{sum(map(int, out[0::2])) / 2**20:.1f}/{sum(map(int, out[1::2])) / 2**20:.0f}")
EOF
}

PID=
start() {
  local root; root=$(root_of "$1")
  case $1 in
    java) (cd "$root" && exec "$JAVA" -jar "$JAR" -w) > "$OUT/server.log" 2>&1 & ;;
    *) (cd "$root" && exec "$BIN" -w) > "$OUT/server.log" 2>&1 & ;;
  esac
  PID=$!
  local i=0
  until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do
    sleep 0.1; i=$((i + 1))
    [ $i -lt 600 ] || { tail "$OUT/server.log"; exit 1; }
  done
}
stop() { kill $PID; wait $PID 2> /dev/null || true; }

BROWSER="Accept-Encoding: gzip, deflate, br, zstd"
UNPACK="Accept: application/vnd.bluemap.bmq3, */*"
# load <run> <server: java|rs|rso (opt-in unpack)|rsn (no script)> <cache: warm|cold> <conns>
load() {
  local run=$1 srv=$2 cache=$3 conns=$4
  local hdr=(--header "$BROWSER"); [ "$srv" = rso ] && hdr+=(--header "$UNPACK")
  local urls=$B mode=(--secs "$SECS")
  { [ "$cache" = cold ] || [ "$srv" = rsn ]; } && mode=(--once)
  [ "$srv" = java ] && [ "$conns" = 1 ] && [ "${mode[0]}" = --once ] && urls=$B1
  start "$srv"
  "$EXE/load_bench" 127.0.0.1:$PORT "$WARM" --conns 32 --secs 3 "${hdr[@]}" > /dev/null
  sleep 3
  local cached=-
  if [ "$cache" = cold ]; then cached=$(evict "$srv"); else warm_cache "$srv"; fi
  local before; before=$(cpu_ticks $PID)
  local out; out=$("$EXE/load_bench" 127.0.0.1:$PORT "$urls" --conns "$conns" "${mode[@]}" "${hdr[@]}")
  local after; after=$(cpu_ticks $PID)
  local rss; rss=$(awk '/VmHWM/ {print $2}' /proc/$PID/status)
  stop
  echo "$out" | python3 -c "
import json, sys
r = json.load(sys.stdin)
n = max(r['requests'], 1)
print(json.dumps({'run': $run, 'server': '$srv', 'cache': '$cache', 'conns': $conns, 'mode': '${mode[0]}',
  'requests': r['requests'], 'rps': r['rps'], 'p50_us': r['p50_us'], 'p99_us': r['p99_us'],
  'cpu_ms': ($after - $before) * 1000 / $TICK / n, 'kb': r['body_bytes'] / n / 1024, 'rss_mb': $rss / 1024,
  'cached_after_evict_mib': '$cached', 'failed': r['requests'] - r['s2xx'] + r['errors']}))" >> "$ROWS"
}

for run in $(seq 1 "$RUNS"); do
  bash ~/devbench-scripts/bgload.sh "before run $run" 2> /dev/null || true
  order="java rs rso"; [ $((run % 2)) -eq 0 ] && order="rso rs java"
  for cache in warm cold; do
    for conns in 1 32 512; do
      for s in $order; do load $run $s $cache $conns; done
    done
    for conns in 1 32; do load $run rsn $cache $conns; done
  done
  echo "run $run done $(date -u +%T) load $(cut -d' ' -f1-3 /proc/loadavg)"
done
bash ~/devbench-scripts/bgload.sh "after" 2> /dev/null || true

python3 - "$ROWS" <<'EOF'
import json, sys
from statistics import median
rows = [json.loads(l) for l in open(sys.argv[1])]
print("| cache | conns | server | mode | tiles | tiles/s | p50 µs | p99 µs | CPU ms/tile | KB/tile | peak RSS MB | cached after evict MiB | failed |")
print("|---|---|---|---|---:|---:|---:|---:|---:|---:|---:|---|---:|")
keys = []
for r in rows:
    k = (r["cache"], r["conns"], r["server"])
    if k not in keys:
        keys.append(k)
for cache, conns, srv in keys:
    g = [r for r in rows if (r["cache"], r["conns"], r["server"]) == (cache, conns, srv)]
    m = lambda k: median([r[k] for r in g])
    rng = lambda k: f"{min(r[k] for r in g):.0f}–{max(r[k] for r in g):.0f}"
    print(f"| {cache} | {conns} | {srv} | {g[0]['mode']} | {m('requests'):.0f} | {m('rps'):.0f} ({rng('rps')}) | {m('p50_us'):.0f} "
          f"| {m('p99_us'):.0f} | {m('cpu_ms'):.3f} ({min(r['cpu_ms'] for r in g):.3f}–{max(r['cpu_ms'] for r in g):.3f}) "
          f"| {m('kb'):.1f} | {m('rss_mb'):.0f} | {' '.join(r['cached_after_evict_mib'] for r in g)} | {max(r['failed'] for r in g)} |")
EOF
rm -rf "$OUT/j" "$OUT/o"
