#!/bin/bash
# usage: [EVENTS=instructions,cycles] [LOCK=1] shapes_instr.sh <checkout>… — encode counters per quad of each
# checkout's built compact_bench (`perf stat` at --reps 4 minus --reps 1, over 3: the one decode cancels), on every
# 16th tile of the real render. Instructions need no lock (they do not depend on load); cycles do (LOCK=1).
TILES=${TILES:-$HOME/bmrs-real/work/real/rs/web/maps/real/tiles/0}
EVENTS=${EVENTS:-instructions}
for co in "$@"; do
  bin=$co/target/release/examples/compact_bench
  run() { ${LOCK:+flock $HOME/bench.lock} perf stat -x, -e "$EVENTS" "$bin" "$TILES" --step 16 --reps "$1" --encode-reps-only 2>&1 >/dev/null; }
  out=$("$bin" "$TILES" --step 16 --reps 1 --encode-reps-only)
  quads=$(echo "$out" | head -1 | grep -o '[0-9.]* M quads' | cut -d' ' -f1)
  r1=$(run 1); r4=$(run 4)
  line="$co: $(echo "$out" | tail -1)"
  for e in ${EVENTS//,/ }; do
    c1=$(echo "$r1" | grep ",$e" | cut -d, -f1); c4=$(echo "$r4" | grep ",$e" | cut -d, -f1)
    line="$line, $(echo "($c4 - $c1) / 3 / ($quads * 1000000)" | bc) $e/quad"
  done
  echo "$line"
done
