#!/bin/bash
# usage: testbox_cpu_ladder.sh <checkout> <fixture dir> <rounds> <ref>… — a commit ladder in one lock per round.
# Builds each ref (profile `profiling`), then per round takes ~/bench.lock once and renders the fixture (`-r -f`) with
# every binary, the order rotated each round; prints user CPU / wall / RSS / instructions:u medians per ref and the
# step to the previous ref. Round 1's webroots are diffed against the first ref's (rstate .dat holds timestamps).
# Keep rounds under ~15 min of lock: refs × one render.
set -e
# run from a copy: checking out a ref may rewrite this file while bash still reads it
if [ -z "$AB_COPY" ]; then cp "$0" /tmp/cpu_ladder.$$.sh; AB_COPY=1 exec bash /tmp/cpu_ladder.$$.sh "$@"; fi
CO=$1; FX=$2; ROUNDS=$3; shift 3; REFS=("$@"); N=${#REFS[@]}
for k in "${!REFS[@]}"; do
  git -C "$CO" checkout -q "${REFS[$k]}" -- .
  (cd "$CO" && ~/.cargo/bin/cargo build -q --profile profiling -p bm-cli 2>&1 | tail -2)
  cp "$CO/target/profiling/bluemap" "$CO/bluemap.$k"
done
git -C "$CO" checkout -q HEAD -- .

cd "$FX"
LOG=$(mktemp)
for r in $(seq "$ROUNDS"); do
  order=""; for i in $(seq 0 $((N - 1))); do order="$order $(( (i + r) % N ))"; done
  flock ~/bench.lock bash -c "for k in $order; do rm -rf web data/logs
      perf stat -x, -e instructions:u -o /tmp/lad.stat.\$\$ /usr/bin/time -f '%U %S %e %M' -o /tmp/lad.time.\$\$ \
        $CO/bluemap.\$k -c config -v 26.3 -r -f >/dev/null 2>&1
      echo \$k \$(cat /tmp/lad.time.\$\$) \$(grep instructions /tmp/lad.stat.\$\$ | cut -d, -f1); rm -f /tmp/lad.*.\$\$
      [ $r = 1 ] && { rm -rf web.\$k; mv web web.\$k; } || true
    done" | tee -a "$LOG"
done
med() { awk -v k=$1 -v c=$2 '$1 == k {print $c}' "$LOG" | sort -g | sed -n "$(( (ROUNDS + 1) / 2 ))p"; }
prev=""
for k in "${!REFS[@]}"; do
  u=$(med $k 2); i=$(med $k 6)
  step=""; [ -n "$prev" ] && step=$(awk -v a="$prev" -v b="$u" -v x="$pi" -v y="$i" 'BEGIN {printf "step user %+.2f%% instr %+.2f%%", (b/a-1)*100, (y/x-1)*100}')
  echo "${REFS[$k]}: user $u s, wall $(med $k 4) s, rss $(med $k 5) kB, instr $i $step"
  prev=$u; pi=$i
  [ $k -gt 0 ] && echo "  non-rstate differences vs ${REFS[0]}: $(diff -rq web.0 web.$k | grep -vc rstate || true)"
done
rm -rf web.* "$LOG"
