#!/bin/bash
# usage: devbox_web_uplink.sh <checkout> <java compat fixture> <ours compat fixture> <optimized fixture> <java> <jar> \
#   <runs> <secs> <scene,server,conns>...
# scene: lo:<cpus> = server pinned to <cpus> logical CPUs, loopback, no limit. veth:<cpus> = veth, MTU 1500, no limit.
#   <mbit>:<cpus> = that veth with tbf on the server's egress at <mbit>. cpus: 8 or 4; the client gets every other CPU.
# server: java | rs (compat) | rso (optimized, unpacking client).
# Runs in a rootless Docker container so veth, tc and netns are private to it: nothing on the host's network changes.
# Same distinct-tile method as devbox_web_distinct.sh; warmup unshaped. Env: OUT, DELAY_MS (added on client->server).
# Run under ~/bench.lock. Results: docs/results/devbox-web-java-vs-rs-2026-10-28.md
set -e
if [ "$1" != inner ]; then
  CO=$1; SRCJ=$2; SRCR=$3; SRCO=$4; JAVA=$5; JAR=$6; shift 6
  OUT=${OUT:-$HOME/jvr-uplink}
  mkdir -p "$OUT"; rm -rf "$OUT/j" "$OUT/r" "$OUT/o"; mkdir -p "$OUT"/{j,r,o}/data
  for p in "j $SRCJ" "r $SRCR" "o $SRCO"; do
    d=${p%% *}; s=${p#* }
    cp -a "$s/config" "$OUT/$d/"; cp -al "$s/web" "$OUT/$d/"
    sed -i -E 's/^(\s*)ip: .*/\1ip: "0.0.0.0"/' "$OUT/$d/config/webserver.conf"
  done
  SELF=$(realpath "$0")
  rc=0
  timeout 3000 docker run --rm --name "bm-uplink-$$" --network none --cap-add NET_ADMIN --cap-add SYS_ADMIN \
    -v /usr:/usr:ro -v /usr/bin:/bin:ro -v /usr/sbin:/sbin:ro -v /usr/lib:/lib:ro -v /usr/lib64:/lib64:ro \
    -v /etc/alternatives:/etc/alternatives:ro -v "$HOME:$HOME" --tmpfs /run --tmpfs /tmp -w "$OUT" -e OUT="$OUT" -e DELAY_MS="${DELAY_MS:-0}" \
    -e BIN="$CO/target/release/bluemap" -e LOAD="$CO/target/profiling/examples/load_bench" -e JAVA="$JAVA" -e JAR="$JAR" \
    --entrypoint /bin/bash hello-world "$SELF" inner "$@" || rc=$?
  rm -rf "$OUT/j" "$OUT/r" "$OUT/o"
  exit $rc
fi
shift
RUNS=$1; SECS=$2; shift 2
ITEMS=("$@")
PORT=8100
ROWS=$OUT/rows.jsonl; : > "$ROWS"
SRV_IP=10.77.0.1
ip netns add srv; ip netns add cli
ip link add vs netns srv type veth peer name vc netns cli
S="nsenter --net=/run/netns/srv"; C="nsenter --net=/run/netns/cli"
$S ip addr add $SRV_IP/30 dev vs; $S ip link set vs up; $S ip link set lo up
$C ip addr add 10.77.0.2/30 dev vc; $C ip link set vc up; $C ip link set lo up
[ "${DELAY_MS:-0}" = 0 ] || $C tc qdisc replace dev vc root netem delay "${DELAY_MS}ms" limit 100000

MAP=$(ls "$OUT/j/web/maps" | head -1)
WARM=$OUT/warmup.txt; B=$OUT/measured.txt
(cd "$OUT/j/web" && find "maps/$MAP/tiles/0" -name '*.prbm.gz' | sed 's/\.gz$//' | sort | shuf --random-source=<(yes)) > "$OUT/hires.txt"
head -1000 "$OUT/hires.txt" > "$WARM"; tail -n +1001 "$OUT/hires.txt" > "$B"
TICK=$(getconf CLK_TCK)
"$JAVA" -version 2>&1 | head -1; "$BIN" --version; uname -r
echo "congestion control: $(cat /proc/sys/net/ipv4/tcp_congestion_control)"; ls -d "$OUT"/{j,r,o}/web/maps/* | head -3

root_of() { case $1 in java) echo "$OUT/j" ;; rs) echo "$OUT/r" ;; *) echo "$OUT/o" ;; esac; }
cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat"; }
tx_bytes() { $S awk -F'[: ]+' '$2 == "vs" {print $11}' /proc/net/dev; }
cpus_srv() { [ "$1" = 4 ] && echo 0-1,8-9 || echo 0-3,8-11; }
cpus_cli() { [ "$1" = 4 ] && echo 2-7,10-15 || echo 4-7,12-15; }
shape() {
  $S tc qdisc del dev vs root 2> /dev/null || true
  # burst: 2 ms of the rate, floor 128 KB so one TSO segment fits; queue: 20 ms
  [ -z "$1" ] || $S tc qdisc replace dev vs root tbf rate "${1}mbit" burst "$((${1} * 250 > 128000 ? ${1} * 250 : 128000))" latency 20ms
}
bg_busy() {
  python3 -c "
import time
r = lambda: [int(x) for x in open('/proc/stat').readline().split()[1:9]]
a = r(); time.sleep(2); b = r(); d = [y - x for x, y in zip(a, b)]
print(f'busy CPUs {(sum(d) - d[3] - d[4]) / sum(d) * 16:.2f}/16, load', open('/proc/loadavg').read().split()[0])"
}

PID=
start() {
  local root; root=$(root_of "$1")
  case $1 in
    java) $S taskset -c "$2" bash -c "cd '$root' && exec '$JAVA' -jar '$JAR' -w" > "$OUT/server.log" 2>&1 & ;;
    *) $S taskset -c "$2" bash -c "cd '$root' && exec '$BIN' -w" > "$OUT/server.log" 2>&1 & ;;
  esac
  PID=$!
  local i=0
  until $C bash -c "exec 3<>/dev/tcp/$SRV_IP/$PORT" 2> /dev/null; do
    sleep 0.1; i=$((i + 1))
    [ $i -lt 600 ] || { tail "$OUT/server.log"; exit 1; }
  done
}
stop() { kill $PID; wait $PID 2> /dev/null || true; }

BROWSER="Accept-Encoding: gzip, deflate, br, zstd"
UNPACK="Accept: application/vnd.bluemap.bmq3, */*"
# row <run> <scene> <server> <conns>
row() {
  local run=$1 scene=$2 srv=$3 conns=$4
  local net=${scene%%:*} cpus=${scene##*:} sc cc ns addr rate=
  sc=$(cpus_srv "$cpus"); cc=$(cpus_cli "$cpus")
  local hdr=(--header "$BROWSER"); [ "$srv" = rso ] && hdr+=(--header "$UNPACK")
  shape ""
  start "$srv" "$sc"
  $C taskset -c "$cc" "$LOAD" $SRV_IP:$PORT "$WARM" --conns 256 --secs 5 "${hdr[@]}" > /dev/null
  sleep 3
  taskset -c "$cc" find "$(root_of "$srv")/web/maps" -type f -exec cat {} + > /dev/null
  case $net in
    lo) ns=$S; addr=127.0.0.1 ;;
    veth) ns=$C; addr=$SRV_IP ;;
    *) ns=$C; addr=$SRV_IP; rate=$net; shape "$net" ;;
  esac
  local ticks0 stat0 tx0 ticks1 stat1 tx1 out rss
  ticks0=$(cpu_ticks $PID); stat0=$(grep '^cpu[0-9]' /proc/stat); tx0=$(tx_bytes)
  out=$($ns taskset -c "$cc" "$LOAD" $addr:$PORT "$B" --conns "$conns" --secs "$SECS" "${hdr[@]}")
  ticks1=$(cpu_ticks $PID); stat1=$(grep '^cpu[0-9]' /proc/stat); tx1=$(tx_bytes)
  rss=$(awk '/VmHWM/ {print $2}' /proc/$PID/status)
  local drops; drops=$($S tc -s qdisc show dev vs | awk '/dropped/ {gsub(",",""); print $7; exit}')
  stop
  OUT_JSON=$out STAT0=$stat0 STAT1=$stat1 python3 -c "
import json, os
r = json.loads(os.environ['OUT_JSON'])
n = max(r['requests'], 1); secs = r['secs']
def busy(cpuset):
    ids = set()
    for part in cpuset.split(','):
        a, _, b = part.partition('-'); ids |= set(range(int(a), int(b or a) + 1))
    def table(s):
        return {int(l.split()[0][3:]): [int(x) for x in l.split()[1:9]] for l in s.splitlines()}
    t0, t1 = table(os.environ['STAT0']), table(os.environ['STAT1'])
    tot = idle = 0
    for i in ids:
        d = [y - x for x, y in zip(t0[i], t1[i])]; tot += sum(d); idle += d[3] + d[4]
    return 100 * (tot - idle) / max(tot, 1)
print(json.dumps({'run': $run, 'scene': '$scene', 'server': '$srv', 'conns': $conns, 'rps': r['rps'],
  'p50_us': r['p50_us'], 'p99_us': r['p99_us'], 'cpu_ms': ($ticks1 - $ticks0) * 1000 / $TICK / n,
  'cpu_pct': ($ticks1 - $ticks0) * 100 / $TICK / secs, 'kb': r['body_bytes'] / n / 1024,
  'goodput_mbit': r['body_bytes'] * 8 / secs / 1e6, 'wire_mbit': ($tx1 - $tx0) * 8 / secs / 1e6,
  'srv_cpus_busy': busy('$sc'), 'cli_cpus_busy': busy('$cc'), 'rss_mb': $rss / 1024,
  'tbf_drops': '${drops:-}', 'failed': r['requests'] - r['s2xx'] + r['errors']}))" >> "$ROWS"
  tail -1 "$ROWS"
}

for run in $(seq 1 "$RUNS"); do
  echo "before run $run: $(bg_busy)"
  items=("${ITEMS[@]}")
  if [ $((run % 2)) -eq 0 ]; then mapfile -t items < <(printf '%s\n' "${ITEMS[@]}" | tac); fi
  for it in "${items[@]}"; do
    IFS=, read -r scene srv conns <<< "$it"
    row "$run" "$scene" "$srv" "$conns"
  done
done
echo "after: $(bg_busy)"

python3 - "$ROWS" <<'EOF'
import json, sys
from statistics import median
rows = [json.loads(l) for l in open(sys.argv[1])]
print("| scene | server | conns | tiles/s (range) | wire Mbit/s | goodput | p50 µs | p99 µs | proc CPU % | ms/tile | srv CPUs busy % | cli CPUs busy % | KB/tile | RSS MB | failed |")
print("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
keys = list(dict.fromkeys((r["scene"], r["server"], r["conns"]) for r in rows))
for k in keys:
    g = [r for r in rows if (r["scene"], r["server"], r["conns"]) == k]
    m = lambda f: median(r[f] for r in g)
    print(f"| {k[0]} | {k[1]} | {k[2]} | {m('rps'):.0f} ({min(r['rps'] for r in g):.0f}–{max(r['rps'] for r in g):.0f}) | {m('wire_mbit'):.0f} | {m('goodput_mbit'):.0f} "
          f"| {m('p50_us'):.0f} | {m('p99_us'):.0f} | {m('cpu_pct'):.0f} | {m('cpu_ms'):.3f} | {m('srv_cpus_busy'):.0f} | {m('cli_cpus_busy'):.0f} "
          f"| {m('kb'):.1f} | {m('rss_mb'):.0f} | {max(r['failed'] for r in g)} |")
EOF
