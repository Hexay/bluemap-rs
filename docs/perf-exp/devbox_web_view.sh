#!/bin/bash
# usage: [ROWS=j528+j529+rc+ro+rn] [NETS=lo+20,50+50,10] [PROXY=<script> FRONT_PORT=<port>] devbox_web_view.sh <checkout> <java fixture> <our compat fixture>
#   <our optimized fixture> <java> <bluemap cli jar> <chrome> [runs=5]
# Time to one view in the real webapp (crates/bm-wasm/bench/view.mjs --reload), Java BlueMap's webserver vs ours.
# Compat rows read the Java render's map files; static webapp files: j528 the Java fixture's, the rest the ours (5.29).
# VIEW_ARGS: extra view.mjs flags, e.g. --no-gl-draw. Every row-run restarts the server and warms it 3 s at 32 conns on tiles outside the view (JIT, not the tiles).
# Run under ~/bench.lock. Raw rows: $OUT/rows.jsonl. Results: docs/results/devbox-web-java-vs-rs-2026-10-28.md
set -e
CO=$1; J=$2; D=$3; O=$4; JAVA=$5; JAR=$6; CHROME=$7; RUNS=${8:-5}
# + separated: testbox.py loses quoting
ROWS_WANTED=${ROWS:-j528+j529+rc+ro+rn}; ROWS_WANTED=${ROWS_WANTED//+/ }
NETS=${NETS:-lo+20,50+50,10}; NETS=${NETS//+/ }
BIN=$CO/target/release/bluemap
LOAD=$CO/target/profiling/examples/load_bench
VIEW=$CO/crates/bm-wasm/bench/view.mjs
OUT=${OUT:-$HOME/jvr-view}
PORT=8100
FRONT=${FRONT_PORT:-$PORT}
URL="http://127.0.0.1:$FRONT/#real:0:70:0:60:0:0.9:0:0:perspective"
mkdir -p "$OUT"; ROWS_FILE=$OUT/rows.jsonl; [ -n "$APPEND" ] || : > "$ROWS_FILE"
ss -ltn | grep -q ":$PORT " && { echo "port $PORT busy"; exit 1; }
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
mkroot j528 "$J" "$J" "$J"
mkroot j529 "$J" "$D" "$J"
mkroot rc "$J" "$D" "$J"
mkroot ro "$O" "$O" "$O"
find "$OUT"/{j528,j529,rc,ro}/web -type f -exec cat {} + > /dev/null

PID=
start() { # <row>
  local root=$1 cmd=("$BIN" -w)
  case $1 in j*) cmd=("$JAVA" -jar "$JAR" -w) ;; rn) root=ro ;; esac
  (cd "$OUT/$root" && exec "${cmd[@]}") > "$OUT/server.log" 2>&1 &
  PID=$!
  local i=0
  until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do
    sleep 0.1; i=$((i + 1))
    [ $i -lt 600 ] || { tail "$OUT/server.log"; exit 1; }
  done
}
stop() { kill $PID; wait $PID 2> /dev/null || true; }
RELAY=
if [ -n "$PROXY" ]; then python3 "$PROXY" "$FRONT" "$PORT" & RELAY=$!; sleep 0.5; fi
trap 'kill $PID $RELAY 2> /dev/null || true' EXIT

view() { # <args...>: prints view.mjs's JSON
  node "$VIEW" "$URL" "$OUT/chrome" --runs 1 --seconds 90 --reload $VIEW_ARGS "$@"
}

# tiles of the view, so that warming never touches them (no-script's transcode cache)
start rc
view | python3 -c "
import json, sys
r = json.load(sys.stdin)['runs'][0]
print('\n'.join(u.lstrip('/') for u in r['network']['tileUrls']))" > "$OUT/view-tiles.txt"
stop
echo "view: $(wc -l < "$OUT/view-tiles.txt") tiles"
(cd "$OUT/rc/web" && find maps/real/tiles/0 -name '*.prbm.gz' | sed 's/\.gz$//' | sort | shuf --random-source=<(yes)) \
  | grep -vxFf "$OUT/view-tiles.txt" | head -4000 > "$OUT/warm.txt"
sha256sum "$OUT"/j529/web/index.html > "$OUT/index.sha"

BROWSER="Accept-Encoding: gzip, deflate, br, zstd"
UNPACK="Accept: application/vnd.bluemap.bmq3, */*"
for run in $(seq 1 "$RUNS"); do
  bash ~/devbench-scripts/bgload.sh "before run $run" 2> /dev/null || true
  rows=$ROWS_WANTED; [ $((run % 2)) -eq 0 ] && rows=$(echo "$rows" | tr ' ' '\n' | tac | tr '\n' ' ')
  nets=$NETS; [ $((run % 2)) -eq 0 ] && nets=$(echo "$nets" | tr ' ' '\n' | tac | tr '\n' ' ')
  for net in $nets; do
    for row in $rows; do
      start "$row"
      hdr=(--header "$BROWSER"); [ "$row" = ro ] && hdr+=(--header "$UNPACK")
      "$LOAD" 127.0.0.1:$PORT "$OUT/warm.txt" --conns 32 --secs 3 "${hdr[@]}" > /dev/null
      sleep 1
      args=(); [ "$net" = lo ] || args+=(--net "$net"); [ "$row" = rn ] && args+=(--no-script)
      out=$(view "${args[@]}")
      load=$(cut -d' ' -f1 /proc/loadavg)
      stop
      echo "$out" | python3 -c "
import json, sys
r = json.load(sys.stdin)['runs'][0]
r.update(run=$run, row='$row', net='$net', load1=$load)
r['network'].pop('tileUrls', None); r.get('reload', {}).get('network', {}).pop('tileUrls', None)
print(json.dumps(r))" >> "$ROWS_FILE"
    done
  done
  echo "run $run done $(date -u +%T) load $(cut -d' ' -f1-3 /proc/loadavg)"
done
bash ~/devbench-scripts/bgload.sh after 2> /dev/null || true
sha256sum -c "$OUT/index.sha"

python3 - "$ROWS_FILE" <<'EOF'
import json, sys
from statistics import median
rows = [json.loads(l) for l in open(sys.argv[1])]
m = lambda g, f: median(f(r) for r in g)
print("| net | row | n | loaded | view ms | main ms | KB all | KB tiles | tile span ms | tile sum ms | tile ms | tile body ms | body>30ms (reused) | max tile | reload ms | reload main | reload KB | reload statuses | reload tile span | load1 |")
print("|---|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---|---:|---:|---:|---:|---|---:|---:|")
for net in dict.fromkeys(r["net"] for r in rows):
    for row in dict.fromkeys(r["row"] for r in rows):
        g = [r for r in rows if r["net"] == net and r["row"] == row]
        if not g:
            continue
        n = lambda f: m(g, lambda r: f(r["network"]) or 0)
        rl = lambda f: m(g, lambda r: f(r["reload"]) or 0)
        print(f"| {net} | {row} | {len(g)} | {sorted({r['loaded'] for r in g})} | {m(g, lambda r: r['ms']):.0f} | {m(g, lambda r: r['mainMs']):.0f} "
              f"| {n(lambda x: x['bytes']) / 1024:.0f} | {n(lambda x: x['tileBytes']) / 1024:.0f} | {n(lambda x: x['tileSpanMs']):.0f} | {n(lambda x: x['tileSumMs']):.0f} "
              f"| {n(lambda x: x['tileMs']):.1f} | {n(lambda x: x['tileBodyMs']):.1f} | {n(lambda x: x['tileBodyOver30']):.0f} ({n(lambda x: x['tileReused']):.0f}) "
              f"| {n(lambda x: x['tileMaxMs']):.0f} | {rl(lambda x: x['ms']):.0f} | {rl(lambda x: x['mainMs']):.0f} "
              f"| {rl(lambda x: x['network']['bytes']) / 1024:.0f} | {g[0]['reload']['network']['statuses']} | {rl(lambda x: x['network']['tileSpanMs']):.0f} "
              f"| {m(g, lambda r: r['load1']):.1f} |")
EOF
