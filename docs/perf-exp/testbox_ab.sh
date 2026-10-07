#!/bin/bash
# usage: testbox_ab.sh <checkout> <patch> [fixture=structures] [runs=3]
# Builds <checkout> (a clone of master) before and after `git apply <patch>`, then renders the fixture with both
# binaries interleaved under ~/bench.lock in a private copy of the fixture, and diffs the two webroots (rstate .dat
# holds timestamps, so only other differences are printed).
set -e
CO=$1; PATCH=$2; FX=${3:-structures}; RUNS=${4:-3}
WORK=$CO/bench/$FX
build() { (cd "$CO" && ~/.cargo/bin/cargo build -q --profile profiling -p bm-cli 2>&1 | tail -2); }

# one command per line: `set -e` ignores failures inside `&&` chains
git -C "$CO" checkout -q -- .
build
cp "$CO/target/profiling/bluemap" "$CO/bluemap.base"
git -C "$CO" apply "$PATCH"
build
cp "$CO/target/profiling/bluemap" "$CO/bluemap.new"
git -C "$CO" checkout -q -- .

rm -rf "$WORK"; mkdir -p "$WORK/data"
cp -r ~/bmrs-eng/bench/$FX/config "$WORK/"
cp -P ~/bmrs-eng/bench/$FX/data/*.jar ~/bmrs-eng/bench/$FX/data/*.zip "$WORK/data/"
cd "$WORK"
for i in $(seq "$RUNS"); do
  for b in base new; do
    flock ~/bench.lock bash -c "rm -rf web data/logs; /usr/bin/time -f '$b user %U wall %e rss %MkB' $CO/bluemap.$b -c config -v 26.3 -r -f 2>&1 >/dev/null | tail -1; rm -rf web.$b; mv web web.$b"
  done
done
echo "non-rstate differences: $(diff -rq web.base web.new | grep -vc rstate || true)"
rm -rf web.base web.new
