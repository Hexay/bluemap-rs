#!/bin/bash
# usage: testbox_flood.sh <checkout> <compat webroot> <optimized webroot of the same map> [secs=10] [mode…]
# Legitimate client-unpack load (32 conns, Accept: bmq3) on the optimized webroot, alone and with a flood of
# distinct hires tiles without the opt-in alongside (docs/18 "Clients that don't unpack"). Each mode (default
# "always limited") is a server with that `--server-unpack`; `none` starts serve_bench without it (an older build).
# FLOODS (default "64 600") = flood connection counts. Run under ~/bench.lock.
set -e
CO=$1; COMPAT=$2; OPT=$3; SECS=${4:-10}
shift $(($# < 4 ? $# : 4)); MODES=${*:-always limited}
EXE=$CO/target/profiling/examples
PORT=8132
(cd "$CO" && ~/.cargo/bin/cargo build -q --profile profiling -p bm-web --examples 2>&1 | tail -3)

MAP=$(ls "$COMPAT/maps" | head -1)
URLS=$(mktemp); LEGIT=$(mktemp); FLOOD=$(mktemp)
(cd "$COMPAT" && find "maps/$MAP/tiles/0" -name '*.prbm.gz' | sed 's/\.gz$//' | sort | shuf --random-source=<(yes)) > "$URLS"
# disjoint halves: the flood never warms the transcode cache for the legitimate client or itself repeating soon
N=$(wc -l < "$URLS"); head -n $((N / 2)) "$URLS" > "$LEGIT"; tail -n +$((N / 2 + 1)) "$URLS" > "$FLOOD"
echo "$N hires tiles of $MAP; load $(cut -d' ' -f1-3 /proc/loadavg), $(nproc) cpus"
find "$OPT/maps/$MAP" -type f -exec cat {} + > /dev/null

BROWSER="Accept-Encoding: gzip, deflate, br, zstd"
UNPACKS="Accept: application/vnd.bluemap.bmq3, */*"
# summary <label> <json>
summary() {
  echo "$2" | python3 -c "
import json, sys
r = json.load(sys.stdin)
n = max(r['requests'], 1)
print(f\"| $1 | {r['rps']:.0f} | {r['p50_us']} | {r['p99_us']} | {r['s2xx']} | {100 * r['s5xx'] / n:.1f} | {r['errors']} |\")"
}

echo "| mode, flood conns: client | req/s | p50 µs | p99 µs | 2xx | 5xx % | errors |"
echo "|---|---|---|---|---|---|---|"
for mode in $MODES; do
  args=(); [ "$mode" != none ] && args=(--server-unpack "$mode")
  "$EXE/serve_bench" "$OPT" --port $PORT --etags --optimized "${args[@]}" > /dev/null 2>&1 &
  pid=$!
  until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do sleep 0.1; done
  out=$("$EXE/load_bench" 127.0.0.1:$PORT "$LEGIT" --conns 32 --secs "$SECS" --header "$BROWSER" --header "$UNPACKS")
  summary "$mode, none: legit" "$out"
  for flood in ${FLOODS:-64 600}; do
    fout=$(mktemp)
    "$EXE/load_bench" 127.0.0.1:$PORT "$FLOOD" --conns "$flood" --secs $((SECS + 2)) --header "$BROWSER" > "$fout" &
    fpid=$!
    sleep 1
    out=$("$EXE/load_bench" 127.0.0.1:$PORT "$LEGIT" --conns 32 --secs "$SECS" --header "$BROWSER" --header "$UNPACKS")
    wait $fpid
    summary "$mode, $flood: legit" "$out"
    summary "$mode, $flood: flood (no opt-in)" "$(cat "$fout")"
    rm -f "$fout"
  done
  kill $pid; wait $pid 2> /dev/null || true
done
rm -f "$URLS" "$LEGIT" "$FLOOD"
