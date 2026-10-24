#!/bin/bash
# usage: shapes_render_ab.sh <base checkout> <new checkout> [runs=3]
# Full `-r -f` render of the 4096² real world into `format: optimized` file storage with both checkouts' bluemap
# (profiling profile), interleaved under ~/bench.lock in a private copy of the fixture config (user time, then
# instructions:u); then diffs the webroots (rstate .dat holds timestamps, so only other differences are counted).
set -e
BASE=$1; NEW=$2; RUNS=${3:-3}
WORK=~/bench-shapes/real-optimized
for co in "$BASE" "$NEW"; do (cd "$co" && ~/.cargo/bin/cargo build -q --profile profiling -p bm-cli 2>&1 | tail -2); done
mkdir -p ~/bench-shapes
cp "$BASE/target/profiling/bluemap" "$WORK.base"
cp "$NEW/target/profiling/bluemap" "$WORK.new"

rm -rf "$WORK"; mkdir -p "$WORK/data"
cp -r ~/bmrs-real/work/bluemap/real/config "$WORK/"
echo 'format: optimized' >> "$WORK/config/storages/file.conf"
cp -P ~/bmrs-real/work/bluemap/real/data/*.jar "$WORK/data/"
cd "$WORK"
for i in $(seq "$RUNS"); do
  for b in base new; do
    flock ~/bench.lock bash -c "rm -rf web data/logs; /usr/bin/time -f '$b user %U sys %S wall %e rss %MkB' perf stat -x, -e instructions:u -o perf.$b $WORK.$b -c config -v 26.3 -r -f 2>&1 >/dev/null | tail -1; grep instructions perf.$b | cut -d, -f1; rm -rf web.$b; mv web web.$b"
  done
done
# bundles hold their tiles in write order, which follows the render threads: compare after converting to compat
for b in base new; do
  rm -rf "conf.$b"; cp -r config "conf.$b"
  sed -i "s#root: \"web/maps\"#root: \"web.$b/maps\"#" "conf.$b/storages/file.conf"
  "$WORK.new" -c "conf.$b" --convert-storage file --to compat >/dev/null 2>&1
done
echo "non-rstate differences after conversion to compat: $(diff -rq web.base web.new | grep -vc rstate || true)"
rm -rf web.base web.new conf.base conf.new
