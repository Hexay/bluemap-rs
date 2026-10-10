#!/bin/bash
# usage: testbox_web.sh <checkout> <compat webroot> <optimized webroot> [secs=5]
# Server CPU per request for hires tiles, compat against optimized storage of the same map: serve_bench on each
# webroot, load_bench walking every tile once in shuffled order (keep conns × secs × req/s below the tile count, or
# the second pass is served from the transcode cache). Linux counterpart of web_bench.py; run under ~/bench.lock.
set -e
CO=$1; COMPAT=$2; OPT=$3; SECS=${4:-5}
EXE=$CO/target/profiling/examples
PORT=8131
(cd "$CO" && ~/.cargo/bin/cargo build -q --profile profiling -p bm-web --examples 2>&1 | tail -3)

MAP=$(ls "$COMPAT/maps" | head -1)
URLS=$(mktemp)
(cd "$COMPAT" && find "maps/$MAP/tiles/0" -name '*.prbm.gz' | sed 's/\.gz$//' | sort | shuf --random-source=<(yes)) > "$URLS"
echo "$(wc -l < "$URLS") hires tiles of $MAP"
# page cache: the numbers are CPU, not disk
find "$COMPAT/maps/$MAP/tiles/0" "$OPT/maps/$MAP" -type f -exec cat {} + > /dev/null
TICK=$(getconf CLK_TCK)
cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat"; }

# run <label> <webroot> <conns> <request header> [serve_bench args…]; UNPACKS=1 also asks for packed tiles
run() {
  local label=$1 root=$2 conns=$3 header=$4; shift 4
  "$EXE/serve_bench" "$root" --port $PORT --etags "$@" > /dev/null 2>&1 &
  local pid=$!
  until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do sleep 0.1; done
  local before; before=$(cpu_ticks $pid)
  local unpacks=(); [ -n "$UNPACKS" ] && unpacks=(--header "Accept: application/vnd.bluemap.bmq3, */*")
  local out; out=$("$EXE/load_bench" 127.0.0.1:$PORT "${URLSET:-$URLS}" --conns "$conns" --secs "$SECS" --header "$header" "${unpacks[@]}")
  local after; after=$(cpu_ticks $pid)
  local rss; rss=$(awk '/VmHWM/ {print $2}' /proc/$pid/status)
  kill $pid; wait $pid 2> /dev/null || true
  echo "$out" | python3 -c "
import json, sys
r = json.load(sys.stdin)
cpu = ($after - $before) * 1000 / $TICK / max(r['requests'], 1)
print(f\"| $label | $conns | {r['rps']:.0f} | {r['p50_us']} | {r['p99_us']} | {cpu:.2f} | {r['body_bytes'] / max(r['requests'], 1) / 1024:.0f} | $((rss / 1024)) | {r['s5xx'] + r['errors']} |\")"
}

# the client script, run in Node against the optimized webroot: every sampled tile must match the compat one
"$EXE/serve_bench" "$OPT" --port $PORT --optimized > /dev/null 2>&1 &
CHECKED=$!
until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do sleep 0.1; done
node "$CO/tools/check_client_unpack.mjs" "http://127.0.0.1:$PORT" "$COMPAT" 500 || echo "CLIENT UNPACK CHECK FAILED"
kill $CHECKED; wait $CHECKED 2> /dev/null || true

BROWSER="Accept-Encoding: gzip, deflate, br, zstd"
echo "| storage, client | conns | req/s | p50 µs | p99 µs | CPU ms/req | KB/req | peak RSS MB | failed |"
echo "|---|---|---|---|---|---|---|---|---|"
for conns in 1 32; do
  run "compat, browser" "$COMPAT" $conns "$BROWSER"
  run "optimized, browser (zstd)" "$OPT" $conns "$BROWSER" --optimized
  run "optimized, gzip only" "$OPT" $conns "Accept-Encoding: gzip" --optimized
  run "optimized, identity" "$OPT" $conns "Accept-Encoding: identity" --optimized
  # the client asks for tiles as stored and unpacks them itself (docs/18)
  UNPACKS=1 run "optimized, client unpacks, browser (zstd)" "$OPT" $conns "$BROWSER" --optimized
  UNPACKS=1 run "optimized, client unpacks, gzip only" "$OPT" $conns "Accept-Encoding: gzip" --optimized
done
# tiles that were served before: the transcode cache holds them
URLSET=$(mktemp); head -200 "$URLS" > "$URLSET"
for conns in 1 32; do
  run "compat, browser, 200 tiles repeated" "$COMPAT" $conns "$BROWSER"
  run "optimized, browser, 200 tiles repeated" "$OPT" $conns "$BROWSER" --optimized
done
rm -f "$URLS" "$URLSET"
