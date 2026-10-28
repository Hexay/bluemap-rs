#!/bin/bash
# usage: testbox_viewer.sh <checkout> <compat webroot> <optimized webroot of the same map> [slots…]
# One viewer without the client script loading a view on the optimized webroot (`server-unpack: limited`, cold
# cache): the 49 hires tiles around the map's center plus 16 lowres tiles, over 6 connections (HTTP/1.1 browser)
# and as one 65-request burst (HTTP/2), per slot count (default "default 1"). docs/18. Run under ~/bench.lock.
set -e
CO=$1; COMPAT=$2; OPT=$3
shift $(($# < 3 ? $# : 3)); SLOTS=${*:-default 1}
EXE=$CO/target/profiling/examples
PORT=8133
(cd "$CO" && ~/.cargo/bin/cargo build -q --profile profiling -p bm-web --examples 2>&1 | tail -3)

MAP=$(ls "$COMPAT/maps" | head -1)
VIEW=$(mktemp)
(cd "$COMPAT" && find "maps/$MAP/tiles/0" "maps/$MAP/tiles/1" -name '*.gz' -o -name '*.png') | python3 -c "
import re, sys
tiles = {0: [], 1: []}
for p in sys.stdin.read().split():
    m = re.match(r'maps/[^/]+/tiles/(\d)/x(-?[\d/]+)z(-?[\d/]+)\.(prbm|png)', p.removesuffix('.gz'))
    if m:
        x, z = (int(v.replace('/', '')) for v in m.group(2, 3))
        tiles[int(m.group(1))].append((max(abs(x), abs(z)), p.removesuffix('.gz')))
xs = [t for _, t in sorted(tiles[0])[:49]] + [t for _, t in sorted(tiles[1])[:16]]
print('\n'.join(xs))" > "$VIEW"
echo "view: $(grep -c prbm "$VIEW") hires + $(grep -c png "$VIEW") lowres of $MAP; load $(cut -d' ' -f1-3 /proc/loadavg)"
echo "| slots | conns | 2xx | 503 | other | wall ms | slowest ms |"
echo "|---|---|---|---|---|---|---|"
for slots in $SLOTS; do
  for conns in 6 65; do
    args=(); [ "$slots" != default ] && args=(--slots "$slots")
    "$EXE/serve_bench" "$OPT" --port $PORT --etags --optimized --server-unpack limited "${args[@]}" > /dev/null 2>&1 &
    pid=$!
    until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do sleep 0.1; done
    out=$("$EXE/load_bench" 127.0.0.1:$PORT "$VIEW" --conns $conns --once --header "Accept-Encoding: gzip, deflate, br, zstd")
    kill $pid; wait $pid 2> /dev/null || true
    echo "$out" | python3 -c "
import json, sys
r = json.load(sys.stdin)
other = r['requests'] - r['s2xx'] - r['s5xx'] + r['errors']
print(f\"| $slots | $conns | {r['s2xx']} | {r['s5xx']} | {other} | {r['secs'] * 1000:.0f} | {r['max_us'] / 1000:.0f} |\")"
  done
done
rm -f "$VIEW"
