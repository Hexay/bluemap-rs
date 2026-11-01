#!/bin/bash
# usage: devbox_web_load_dump.sh <checkout> <fixtures dir with j12 d12 d12-opt> <dir with bluemap> <java> <jar> <chrome> [rows=j529+rc+ro]
# What one first load and one reload of the real webapp's default view consist of, per request (view.mjs --dump +
# load_breakdown.mjs). Roots as in devbox_web_view.sh, under $OUT; the map tiles are hard links, nothing else is shared.
# Run under ~/bench.lock. Results: docs/results/devbox-webapp-load-breakdown-2026-11-01.md
set -e
CO=$1; F=$2; BIN=$3/bluemap; JAVA=$4; JAR=$5; CHROME=$6; ROWS=${7:-j529+rc+ro}; ROWS=${ROWS//+/ }
OUT=${OUT:-$HOME/viewtrace-load}
PORT=8100
URL="http://127.0.0.1:$PORT/#real:0:70:0:60:0:0.9:0:0:perspective"
BENCH=$CO/crates/bm-wasm/bench
ss -ltn | grep -q ":$PORT " && { echo "port $PORT busy"; exit 1; }
mkdir -p "$OUT"
cp "$BIN" "$OUT/bluemap"
printf '#!/bin/sh\nexec "%s" --no-sandbox "$@"\n' "$CHROME" > "$OUT/chrome"; chmod +x "$OUT/chrome"

# mkroot <name> <config from> <static from> <maps from>: static files copied, map tiles hard-linked
mkroot() {
  local r=$OUT/$1
  rm -rf "$r"; mkdir -p "$r/web" "$r/data"
  cp -a "$2/config" "$r/"
  (cd "$3/web" && cp -a $(ls | grep -v '^maps$') "$r/web/")
  cp -al "$4/web/maps" "$r/web/"
  find "$r/web/maps" -maxdepth 2 -type f -exec sh -c 'cp --remove-destination "$(readlink -f "$0")" "$0.tmp" && mv "$0.tmp" "$0"' {} \;
}
PID=
trap 'kill $PID 2> /dev/null || true' EXIT
for row in $ROWS; do
  case $row in
    j529) mkroot j529 "$F/j12" "$F/d12" "$F/j12"; cmd=("$JAVA" -jar "$JAR" -w) ;;
    rc) mkroot rc "$F/j12" "$F/d12" "$F/j12"; cmd=("$OUT/bluemap" -w) ;;
    ro) mkroot ro "$F/d12-opt" "$F/d12-opt" "$F/d12-opt"; cmd=("$OUT/bluemap" -w) ;;
  esac
  (cd "$OUT/$row" && exec "${cmd[@]}") > "$OUT/$row.log" 2>&1 &
  PID=$!
  until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do sleep 0.1; done
  rm -f "$OUT/$row.jsonl"
  node "$BENCH/view.mjs" "$URL" "$OUT/chrome" --runs 1 --seconds 90 --reload --dump "$OUT/$row.jsonl" \
    | python3 -c "import json,sys; r=json.load(sys.stdin)['runs'][0]; n=r['network']; print('$row', r['loaded'], n['requests'], n['statuses'], n['bytes'], n['tileBytes'], 'reload', r['reload']['network']['statuses'], r['reload']['network']['bytes'])"
  echo "=== $row"
  node "$BENCH/load_breakdown.mjs" "$OUT/$row.jsonl" --fetch "http://127.0.0.1:$PORT/" | tee "$OUT/$row.md"
  kill $PID; wait $PID 2> /dev/null || true
done
