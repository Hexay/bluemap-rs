#!/bin/bash
# usage: devbox_web_java_vs_rs.sh <checkout> <java-rendered fixture dir (config/ + web/)> <java> <bluemap cli jar> [runs=3] [secs=5]
# Java BlueMap's webserver (`-w`) against ours (`bluemap -w`, the checkout's target/release) on one hard-linked copy
# of the fixture: same config (the generated default webserver.conf), same files. load_bench drives both; server CPU
# from /proc/<pid>/stat, peak RSS from VmHWM; every row restarts the server and warms it 3 s at 32 conns first (JIT).
# Rows are interleaved Java/ours, order flipped each run; raw rows go to $OUT/rows.jsonl, medians to stdout.
# Run under ~/bench.lock. Results: docs/results/devbox-web-java-vs-rs-2026-10-28.md
set -e
CO=$1; SRC=$2; JAVA=$3; JAR=$4; RUNS=${5:-3}; SECS=${6:-5}
EXE=$CO/target/profiling/examples
BIN=$CO/target/release/bluemap
HERE=$(cd "$(dirname "$0")" && pwd)
OUT=${OUT:-$HOME/jvr-web}
ROOT=$OUT/root
PORT=8100
mkdir -p "$OUT"; rm -rf "$ROOT"; mkdir -p "$ROOT"
cp -a "$SRC/config" "$ROOT/"; cp -al "$SRC/web" "$ROOT/"; mkdir -p "$ROOT/data"
ROWS=$OUT/rows.jsonl; : > "$ROWS"
ss -ltn | grep -q ":$PORT " && { echo "port $PORT busy"; exit 1; }

MAP=$(ls "$ROOT/web/maps" | head -1)
URLS=$OUT/hires.txt; URLSET=$OUT/hires200.txt; LOWRES=$OUT/lowres.txt; JSONS=$OUT/json.txt
(cd "$ROOT/web" && find "maps/$MAP/tiles/0" -name '*.prbm.gz' | sed 's/\.gz$//' | sort | shuf --random-source=<(yes)) > "$URLS"
head -200 "$URLS" > "$URLSET"
(cd "$ROOT/web" && find "maps/$MAP/tiles" -name '*.png' | sort | shuf --random-source=<(yes)) > "$LOWRES"
printf 'maps/%s/settings.json\nmaps/%s/textures.json\n' "$MAP" "$MAP" > "$JSONS"
echo "$(wc -l < "$URLS") hires, $(wc -l < "$LOWRES") lowres tiles of $MAP"
find "$ROOT/web" -type f -exec cat {} + > /dev/null
TICK=$(getconf CLK_TCK)
cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat"; }
"$JAVA" -version 2>&1 | head -1
"$JAVA" -XX:+PrintFlagsFinal -version 2>/dev/null | grep -E ' (MaxHeapSize|UseG1GC|UseSerialGC|ActiveProcessorCount) '
"$BIN" --version

PID=
start() {
  case $1 in
    java) (cd "$ROOT" && exec "$JAVA" -jar "$JAR" -w) > "$OUT/server.log" 2>&1 & ;;
    rs) (cd "$ROOT" && exec "$BIN" -w) > "$OUT/server.log" 2>&1 & ;;
  esac
  PID=$!
  local i=0
  until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do
    sleep 0.1; i=$((i + 1))
    [ $i -lt 600 ] || { tail "$OUT/server.log"; exit 1; }
  done
}
stop() { kill $PID; wait $PID 2> /dev/null || true; }

# load <run> <server> <label> <conns> <header> <urls>
load() {
  local run=$1 srv=$2 label=$3 conns=$4 header=$5 urls=$6
  start "$srv"
  "$EXE/load_bench" 127.0.0.1:$PORT "$urls" --conns 32 --secs 3 --header "$header" > /dev/null
  # background CPU after the warmup (JIT, GC), subtracted from cpu_net_ms: at 1 conn Java serves only ~30 req/s
  local idle0; idle0=$(cpu_ticks $PID); sleep 3; local idle1; idle1=$(cpu_ticks $PID)
  local before; before=$(cpu_ticks $PID)
  local out; out=$("$EXE/load_bench" 127.0.0.1:$PORT "$urls" --conns "$conns" --secs "$SECS" --header "$header")
  local after; after=$(cpu_ticks $PID)
  local rss; rss=$(awk '/VmHWM/ {print $2}' /proc/$PID/status)
  local threads; threads=$(ls /proc/$PID/task | wc -l)
  stop
  echo "$out" | python3 -c "
import json, sys
r = json.load(sys.stdin)
n = max(r['requests'], 1)
print(json.dumps({'kind': 'load', 'run': $run, 'server': '$srv', 'label': '$label', 'conns': $conns,
  'rps': r['rps'], 'p50_us': r['p50_us'], 'p99_us': r['p99_us'], 'cpu_ms': ($after - $before) * 1000 / $TICK / n,
  'idle_cpu_ms_per_s': ($idle1 - $idle0) * 1000 / $TICK / 3,
  'cpu_net_ms': (($after - $before) - ($idle1 - $idle0) * r['secs'] / 3) * 1000 / $TICK / n,
  'kb': r['body_bytes'] / n / 1024, 'rss_mb': $rss / 1024, 'threads': $threads,
  'failed': r['requests'] - r['s2xx'] + r['errors']}))" >> "$ROWS"
}

reload() {
  local run=$1 srv=$2
  start "$srv"
  local out; out=$(python3 "$HERE/web_reload.py" 127.0.0.1:$PORT "$ROOT/web" "$MAP" --pid $PID)
  stop
  echo "$out" | python3 -c "
import json, sys
r = json.load(sys.stdin); r.update(kind='reload', run=$run, server='$srv'); print(json.dumps(r))" >> "$ROWS"
}

GZ="Accept-Encoding: gzip"; BROWSER="Accept-Encoding: gzip, deflate, br, zstd"; Q="Accept-Encoding: gzip;q=1.0"
for run in $(seq 1 "$RUNS"); do
  bash ~/devbench-scripts/bgload.sh "before run $run" 2> /dev/null || true
  order="java rs"; [ $((run % 2)) -eq 0 ] && order="rs java"
  for conns in 1 32; do
    for s in $order; do load $run $s "hires, gzip" $conns "$GZ" "$URLS"; done
    for s in $order; do load $run $s "hires, browser" $conns "$BROWSER" "$URLS"; done
    for s in $order; do load $run $s "hires, gzip;q=1.0" $conns "$Q" "$URLS"; done
    for s in $order; do load $run $s "hires, 200 repeated, browser" $conns "$BROWSER" "$URLSET"; done
    for s in $order; do load $run $s "lowres png, browser" $conns "$BROWSER" "$LOWRES"; done
  done
  # Java's 1-32 conn rows sit on a ~40 ms per-response stall; enough connections show its CPU-bound ceiling
  for s in $order; do load $run $s "hires, browser" 512 "$BROWSER" "$URLS"; done
  for s in $order; do load $run $s "settings+textures.json, browser" 1 "$BROWSER" "$JSONS"; done
  for s in $order; do reload $run $s; done
  echo "run $run done $(date -u +%T) load $(cut -d' ' -f1-3 /proc/loadavg)"
done
bash ~/devbench-scripts/bgload.sh "after" 2> /dev/null || true

python3 - "$ROWS" <<'EOF'
import json, sys
from statistics import median
rows = [json.loads(l) for l in open(sys.argv[1])]
med = lambda xs: median(xs)
print("| row | conns | server | req/s | p50 µs | p99 µs | CPU ms/req | net CPU ms/req | idle CPU ms/s | KB/req | peak RSS MB | threads | failed |")
print("|---|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
keys = []
for r in rows:
    if r["kind"] == "load" and (r["label"], r["conns"]) not in keys:
        keys.append((r["label"], r["conns"]))
for label, conns in keys:
    for srv in ("java", "rs"):
        g = [r for r in rows if r["kind"] == "load" and r["label"] == label and r["conns"] == conns and r["server"] == srv]
        m = lambda k: med([r[k] for r in g])
        print(f"| {label} | {conns} | {srv} | {m('rps'):.0f} | {m('p50_us'):.0f} | {m('p99_us'):.0f} | {m('cpu_ms'):.3f} "
              f"| {m('cpu_net_ms'):.3f} | {m('idle_cpu_ms_per_s'):.1f} "
              f"| {m('kb'):.1f} | {m('rss_mb'):.0f} | {m('threads'):.0f} | {max(r['failed'] for r in g)} |")
print()
print("| pass | server | validators | status | wall ms (median) | wire KB/view | hires wire KB/tile | CPU ms/req |")
print("|---|---|---|---|---:|---:|---:|---:|")
for kind in ("full", "reload"):
    for srv in ("java", "rs"):
        g = [r for r in rows if r["kind"] == "reload" and r["server"] == srv]
        p = [r[kind] for r in g]
        print(f"| {kind} | {srv} | {g[0]['validators']} | {p[0]['status']} | {med([x['wall_ms_median'] for x in p]):.1f} "
              f"| {med([x['wire_kb_per_pass'] for x in p]):.1f} | {med([x['hires_wire_kb_per_tile'] for x in p]):.2f} "
              f"| {med([x['cpu_ms_per_request'] for x in p]):.3f} |")
EOF
rm -rf "$ROOT"
