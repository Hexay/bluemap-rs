#!/bin/bash
# usage: [NETS=20,50+50,10] [RUNS=5] [LOAD_ARGS=--trace] devbox_load_ab.sh <checkout> <optimized fixture> <out dir> <label=script.js[,wasm]>...
# A/B of the client script on one server build: crates/bm-wasm/bench/load_ab.mjs per net/variant/run, interleaved, through
# mss_relay.py + CDP shaping. label=- keeps the script the server embeds. Run under ~/bench.lock. Rows: <out dir>/rows.jsonl.
set -e
CO=$1; FIX=$2; OUT=$3; shift 3
RUNS=${RUNS:-5}; NETS=${NETS:-20,50+50,10}; NETS=${NETS//+/ }; LOAD_ARGS=${LOAD_ARGS//+/ } # + separated: testbox.py loses quoting
BIN=$CO/target/release/bluemap
LOAD=$HOME/bmrs-devbench/target/profiling/examples/load_bench
TOOL=$CO/crates/bm-wasm/bench/load_ab.mjs
CHROME_REAL=$HOME/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome
PORT=8100; FRONT=8101
URL="http://127.0.0.1:$FRONT/#real:0:70:0:60:0:0.9:0:0:perspective"
mkdir -p "$OUT"
printf '#!/bin/sh\nexec "%s" --no-sandbox "$@"\n' "$CHROME_REAL" > "$OUT/chrome"; chmod +x "$OUT/chrome"
ss -ltn | grep -q ":$PORT " && { echo "port $PORT busy"; exit 1; }

R=$OUT/root
if [ ! -d "$R/web/maps" ]; then
  rm -rf "$R"; mkdir -p "$R/web" "$R/data"
  cp -a "$FIX/config" "$R/"
  (cd "$FIX/web" && cp -a $(ls | grep -v '^maps$') "$R/web/")
  cp -al "$FIX/web/maps" "$R/web/"
  # top-level map files get rewritten by a server start: never through the hard link
  find "$R/web/maps" -maxdepth 2 -type f -exec sh -c 'cp --remove-destination "$(readlink -f "$0")" "$0.tmp" && mv "$0.tmp" "$0"' {} \;
fi
[ -f "$OUT/warm.txt" ] || (cd "$FIX/../d12/web" && find maps/real/tiles/0 -name '*.prbm.gz' | sed 's/\.gz$//') | python3 -c "
import random, re, sys
far = [l.strip() for l in sys.stdin if int(re.search(r'/x(-?[\d/]+)/z', l).group(1).replace('/', '')) < -30]
random.Random(1).shuffle(far)
print('\n'.join(far[:4000]))" > "$OUT/warm.txt"
find "$R/web" -type f -exec cat {} + > /dev/null

PID=
python3 "$CO/docs/perf-exp/mss_relay.py" $FRONT $PORT & RELAY=$!
trap 'kill $PID $RELAY 2> /dev/null || true' EXIT
start() {
  (cd "$R" && exec "$BIN" -w) > "$OUT/server.log" 2>&1 &
  PID=$!
  local i=0
  until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do
    sleep 0.1; i=$((i + 1))
    [ $i -lt 600 ] || { tail "$OUT/server.log"; exit 1; }
  done
  "$LOAD" 127.0.0.1:$PORT "$OUT/warm.txt" --conns 32 --secs 2 --header "Accept-Encoding: gzip, deflate, br, zstd" \
    --header "Accept: application/vnd.bluemap.bmq3, */*" > /dev/null
  sleep 0.5
}
stop() { kill $PID; wait $PID 2> /dev/null || true; }

for run in $(seq 1 "$RUNS"); do
  variants=("$@"); [ $((run % 2)) -eq 0 ] && variants=($(printf '%s\n' "$@" | tac))
  for net in $NETS; do
    for v in "${variants[@]}"; do
      label=${v%%=*}; spec=${v#*=}
      # label=script,wasm[,<tuning json without spaces>]
      IFS=, read -r s w tune <<< "$spec"
      a=(); [ "$spec" = - ] || a=(--script "$s" --wasm "$w")
      [ -n "$tune" ] && a+=(--tune "$tune")
      start
      timeout 240 node "$TOOL" "$URL" "$OUT/chrome" --label "$label" --net "$net" --run "$run" --out "$OUT/rows.jsonl" "${a[@]}" $LOAD_ARGS \
        || echo "$label $net run $run: tool failed"
      stop
    done
  done
  echo "run $run done $(date -u +%T) load $(cut -d' ' -f1-3 /proc/loadavg)"
done
