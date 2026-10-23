#!/bin/bash
# usage: testbox_cpu_ladder.sh <checkout> <fixture dir> <rounds> <ref>… — a commit ladder in one lock per round.
# Builds each ref (profile `profiling`), then per round takes ~/bench.lock once and renders the fixture (`-r -f`) with
# every binary, the order rotated each round; prints user CPU / wall / RSS / instructions:u / cycles:u medians per
# ref and the step to the previous ref. Round 1's webroots are diffed against the first ref's: optimized bundles hold
# records in completion order, so with CONVERT=1 each is first converted to compat (`--convert-storage file`).
# rstate .dat holds timestamps and is left out. Keep rounds under ~15 min of lock: refs × one render.
# NOOP=1 times a no-op update instead: one forced render first, then every run is `-r` over that webroot.
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
ARGS="-c config -v 26.3 -r -f"; CLEAN="rm -rf web data/logs"
if [ -n "$NOOP" ]; then
  rm -rf web data/logs; "$CO/bluemap.0" $ARGS >/dev/null 2>&1
  ARGS="-c config -v 26.3 -r"; CLEAN="rm -rf data/logs"
fi
for r in $(seq "$ROUNDS"); do
  order=""; for i in $(seq 0 $((N - 1))); do order="$order $(( (i + r) % N ))"; done
  flock ~/bench.lock bash -c "for k in $order; do $CLEAN
      perf stat -x, -e instructions:u,cycles:u -o /tmp/lad.stat.\$\$ /usr/bin/time -f '%U %S %e %M' -o /tmp/lad.time.\$\$ \
        $CO/bluemap.\$k $ARGS >/dev/null 2>&1
      echo \$k \$(cat /tmp/lad.time.\$\$) \$(grep -E 'instructions|cycles' /tmp/lad.stat.\$\$ | cut -d, -f1); rm -f /tmp/lad.*.\$\$
      [ $r = 1 ] && [ -z '$NOOP' ] && { rm -rf web.\$k; mv web web.\$k; } || true
    done" | tee -a "$LOG"
done
[ -n "$NOOP" ] && for k in "${!REFS[@]}"; do mkdir -p web.$k; done
if [ -n "$CONVERT" ]; then
  # the conversion also rewrites the storage config's `format`
  cp config/storages/file.conf /tmp/lad.conf.$$
  for k in "${!REFS[@]}"; do
    rm -rf web; mv web.$k web; "$CO/bluemap.0" -c config --convert-storage file --to compat >/dev/null 2>&1; mv web web.$k
    cp /tmp/lad.conf.$$ config/storages/file.conf
  done
  rm -f /tmp/lad.conf.$$
fi
med() { awk -v k=$1 -v c=$2 '$1 == k {print $c}' "$LOG" | sort -g | sed -n "$(( (ROUNDS + 1) / 2 ))p"; }
pct() { awk -v a="$1" -v b="$2" 'BEGIN {printf "%+.2f%%", (b/a-1)*100}'; }
for k in "${!REFS[@]}"; do
  u=$(med $k 2); i=$(med $k 6); c=$(med $k 7)
  step=""; [ $k -gt 0 ] && step="step user $(pct $pu $u) instr $(pct $pi $i) cycles $(pct $pc $c)"
  echo "${REFS[$k]}: user $u s, wall $(med $k 4) s, rss $(med $k 5) kB, instr $i, cycles $c $step"
  pu=$u; pi=$i; pc=$c
  [ $k -gt 0 ] && echo "  non-rstate differences vs ${REFS[0]}: $(diff -rq web.0 web.$k | grep -vc rstate || true)"
done
rm -rf web.* "$LOG"
