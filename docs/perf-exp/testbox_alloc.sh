#!/bin/bash
# usage: testbox_alloc.sh <dir with bluemap.<variant> binaries> [fixture=structures-opt] [runs=3] [variants...]
# Forced full render of a ~/bmrs-eng/bench fixture with each prebuilt binary (e.g. glibc / musl / musl+mimalloc
# cross-builds from tools/build_core.py), interleaved under ~/bench.lock; prints user/sys/wall/peak RSS per run and
# checks that every variant wrote the same webroot (rstate .dat holds timestamps, so those are ignored).
set -e
BIN=$1; FX=${2:-structures-opt}; RUNS=${3:-3}; shift $(( $# < 3 ? $# : 3 ))
VARIANTS=${*:-glibc musl mimalloc}
WORK=$BIN/bench/$FX

rm -rf "$WORK"; mkdir -p "$WORK/data"
cp -r ~/bmrs-eng/bench/$FX/config "$WORK/"
cp -P ~/bmrs-eng/bench/$FX/data/*.jar ~/bmrs-eng/bench/$FX/data/*.zip "$WORK/data/"
cd "$WORK"
for i in $(seq "$RUNS"); do
  for b in $VARIANTS; do
    flock ~/bench.lock bash -c "rm -rf web data/logs; /usr/bin/time -f '$b user %U sys %S wall %e rss %MkB' $BIN/bluemap.$b -c config -v 26.3 -r -f 2>&1 >/dev/null | tail -1; rm -rf web.$b; mv web web.$b"
  done
done
first=${VARIANTS%% *}
for b in $VARIANTS; do
  [ "$b" = "$first" ] && continue
  echo "$first vs $b non-rstate differences: $(diff -rq web.$first web.$b | grep -vc rstate || true)"
done
rm -rf web.*
