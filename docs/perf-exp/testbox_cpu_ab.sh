#!/bin/bash
# usage: testbox_cpu_ab.sh <checkout> <fixture dir> [runs=3] [base-ref=HEAD~1] [new-ref=HEAD]
# testbox_ab.sh for any fixture dir (config/ + data/, e.g. ~/bmrs-cpu/bench/real-opt) and two commits (after
# `testbox.py sync`, HEAD is the synced tree and HEAD~1 the local HEAD): builds both, renders interleaved under
# ~/bench.lock with user CPU and `instructions:u` per run, prints the medians, then diffs the two webroots (rstate
# .dat holds timestamps, so only other differences are printed).
set -e
# run from a copy: checking out the base ref may rewrite this file while bash still reads it
if [ -z "$AB_COPY" ]; then cp "$0" /tmp/cpu_ab.$$.sh; AB_COPY=1 exec bash /tmp/cpu_ab.$$.sh "$@"; fi
CO=$1; FX=$2; RUNS=${3:-3}; BASE=${4:-HEAD~1}; NEW=${5:-HEAD}
build() { (cd "$CO" && ~/.cargo/bin/cargo build -q --profile profiling -p bm-cli 2>&1 | tail -2); }

for b in base new; do
  ref=$BASE; [ $b = new ] && ref=$NEW
  git -C "$CO" checkout -q "$ref" -- .
  build
  cp "$CO/target/profiling/bluemap" "$CO/bluemap.$b"
done
git -C "$CO" checkout -q HEAD -- .

cd "$FX"
LOG=$(mktemp)
for i in $(seq "$RUNS"); do
  for b in base new; do
    flock ~/bench.lock bash -c "rm -rf web data/logs
      perf stat -x, -e instructions:u -o /tmp/ab.stat.\$\$ /usr/bin/time -f '%U %S %e %M' -o /tmp/ab.time.\$\$ \
        $CO/bluemap.$b -c config -v 26.3 -r -f >/dev/null 2>&1
      echo $b \$(cat /tmp/ab.time.\$\$) \$(grep instructions /tmp/ab.stat.\$\$ | cut -d, -f1); rm -f /tmp/ab.*.\$\$
      rm -rf web.$b; mv web web.$b" | tee -a "$LOG"
  done
done
med() { awk -v b=$1 -v c=$2 '$1 == b {print $c}' "$LOG" | sort -g | sed -n "$(( (RUNS + 1) / 2 ))p"; }
for b in base new; do
  echo "$b median: user $(med $b 2) s, wall $(med $b 4) s, rss $(med $b 5) kB, instructions:u $(med $b 6)"
done
echo "non-rstate differences: $(diff -rq web.base web.new | grep -vc rstate || true)"
rm -rf web.base web.new "$LOG"
