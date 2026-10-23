#!/bin/bash
# usage: shapes_instr.sh <checkout>… — encode instructions per quad of each checkout's built compact_bench
# (`perf stat` at --reps 4 minus --reps 1, over 3), on every 16th tile of the real render. Needs no lock: instruction
# counts do not depend on load.
TILES=${TILES:-$HOME/bmrs-real/work/real/rs/web/maps/real/tiles/0}
for co in "$@"; do
  bin=$co/target/release/examples/compact_bench
  count() { perf stat -x, -e instructions "$bin" "$TILES" --step 16 --reps "$1" --encode-reps-only 2>&1 >/dev/null | grep instructions | cut -d, -f1; }
  out=$("$bin" "$TILES" --step 16 --reps 1 --encode-reps-only)
  quads=$(echo "$out" | head -1 | grep -o '[0-9.]* M quads' | cut -d' ' -f1)
  i1=$(count 1); i4=$(count 4)
  echo "$co: $(( (i4 - i1) / 3 / 1000000 )) M instr/rep, $(echo "($i4 - $i1) / 3 / ($quads * 1000000)" | bc) instr/quad, $(echo "$out" | tail -1)"
done
