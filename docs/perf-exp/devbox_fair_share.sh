#!/bin/bash
# usage: devbox_fair_share.sh <checkout> <compat webroot> <optimized webroot of the same map> <base rev> [secs=10]
# `server-unpack` fair share (docs/18): a closed-loop flood without the script from one client while another loads
# a view without the script (the 49 hires tiles around the center + 16 lowres, cold cache, over 6 connections and
# as one 65-request burst); and script clients alone and under the flood. Clients are told apart by
# X-Forwarded-For (flood 203.0.113.1, viewer 203.0.113.2) through a trusted 127.0.0.1. "base" is <base rev> built
# in a scratch worktree (it ignores the header: one client). FLOODS (default "64 600"), REPS (default 3).
# Run under ~/bench.lock.
set -e
CO=$1; COMPAT=$2; OPT=$3; BASE_REV=$4; SECS=${5:-10}
PORT=8134
BASE=$CO/target/fair-base
build() { (cd "$1" && CARGO_TARGET_DIR=$2 ~/.cargo/bin/cargo build -q --profile profiling -p bm-web --examples 2>&1 | tail -3); }
git -C "$CO" worktree remove --force "$BASE" 2> /dev/null || rm -rf "$BASE"
git -C "$CO" worktree prune
git -C "$CO" worktree add -q --detach "$BASE" "$BASE_REV"
build "$BASE" "$CO/target/fair-base-target"
build "$CO" "$CO/target"
NEW=$CO/target/profiling/examples
OLD=$CO/target/fair-base-target/profiling/examples
LOAD=$NEW/load_bench

MAP=$(ls "$COMPAT/maps" | head -1)
VIEW=$(mktemp); FLOOD=$(mktemp); SCRIPT=$(mktemp)
(cd "$COMPAT" && find "maps/$MAP/tiles/0" "maps/$MAP/tiles/1" -name '*.gz' -o -name '*.png') | python3 -c "
import re, sys
tiles = {0: [], 1: []}
for p in sys.stdin.read().split():
    m = re.match(r'maps/[^/]+/tiles/(\d)/x(-?[\d/]+)z(-?[\d/]+)\.(prbm|png)', p.removesuffix('.gz'))
    if m:
        x, z = (int(v.replace('/', '')) for v in m.group(2, 3))
        tiles[int(m.group(1))].append((max(abs(x), abs(z)), p.removesuffix('.gz')))
hires = [t for _, t in sorted(tiles[0])]
open('$VIEW', 'w').write('\n'.join(hires[:49] + [t for _, t in sorted(tiles[1])[:16]]))
rest = hires[49:]
import random; random.Random(1).shuffle(rest)
open('$FLOOD', 'w').write('\n'.join(rest))
open('$SCRIPT', 'w').write('\n'.join(hires))"
echo "view: $(grep -c prbm "$VIEW") hires + $(grep -c png "$VIEW") lowres of $MAP; flood: $(wc -l < "$FLOOD") others"
echo "load $(cut -d' ' -f1-3 /proc/loadavg), $(nproc) cpus, base $(git -C "$BASE" log --oneline -1)"
find "$OPT/maps/$MAP" -type f -exec cat {} + > /dev/null

BROWSER="Accept-Encoding: gzip, deflate, br, zstd"
UNPACKS="Accept: application/vnd.bluemap.bmq3+zstd, application/vnd.bluemap.bmq3, */*"
pid=
start() {
  local args=(--etags --optimized --server-unpack limited)
  [ "$1" = new ] && args+=(--trusted-proxies 127.0.0.1)
  local exe=$OLD; [ "$1" = new ] && exe=$NEW
  "$exe/serve_bench" "$OPT" --port $PORT "${args[@]}" > /dev/null 2>&1 &
  pid=$!
  until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do sleep 0.1; done
}
stop() { kill $pid; wait $pid 2> /dev/null || true; }
field() { python3 -c "import json, sys; r = json.loads(sys.argv[1]); print($2)" "$1"; }

echo
echo "| build | flood conns | script req/s | script p50 µs | script p99 µs |"
echo "|---|---|---|---|---|"
for build in base new; do
  start $build
  for flood in none ${FLOODS:-64 600}; do
    fpid=
    if [ "$flood" != none ]; then
      "$LOAD" 127.0.0.1:$PORT "$FLOOD" --conns "$flood" --secs $((SECS + 2)) --header "$BROWSER" \
        --header "X-Forwarded-For: 203.0.113.1" > /dev/null &
      fpid=$!
      sleep 1
    fi
    out=$("$LOAD" 127.0.0.1:$PORT "$SCRIPT" --conns 32 --secs "$SECS" --header "$BROWSER" --header "$UNPACKS" \
      --header "X-Forwarded-For: 203.0.113.3")
    [ -n "$fpid" ] && wait $fpid
    echo "| $build | $flood | $(field "$out" "f\"{r['rps']:.0f} | {r['p50_us']} | {r['p99_us']}\"") |"
  done
  stop
done

echo
echo "| build | flood conns | viewer conns | viewer 2xx | viewer 503 | load wall ms | slowest ms | flood req/s | flood 503 % |"
echo "|---|---|---|---|---|---|---|---|---|"
for build in base new; do
  for flood in ${FLOODS:-64 600}; do
    for conns in 6 65; do
      for rep in $(seq "${REPS:-3}"); do
        start $build
        fout=$(mktemp)
        "$LOAD" 127.0.0.1:$PORT "$FLOOD" --conns "$flood" --secs 4 --header "$BROWSER" \
          --header "X-Forwarded-For: 203.0.113.1" > "$fout" &
        fpid=$!
        sleep 1.5
        out=$("$LOAD" 127.0.0.1:$PORT "$VIEW" --conns $conns --once --header "$BROWSER" \
          --header "X-Forwarded-For: 203.0.113.2")
        wait $fpid
        stop
        f=$(cat "$fout"); rm -f "$fout"
        echo "| $build | $flood | $conns | $(field "$out" "f\"{r['s2xx']} | {r['s5xx']} | {r['secs'] * 1000:.0f} | {r['max_us'] / 1000:.0f}\"") | $(field "$f" "f\"{r['rps']:.0f} | {100 * r['s5xx'] / max(r['requests'], 1):.0f}\"") |"
      done
    done
  done
done
echo "load $(cut -d' ' -f1-3 /proc/loadavg)"
rm -f "$VIEW" "$FLOOD" "$SCRIPT"
git -C "$CO" worktree remove --force "$BASE"
