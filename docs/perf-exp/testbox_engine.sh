#!/bin/bash
# usage: eng.sh <plain|perf|heap|timeline> [fixture dir under ~/bmrs-eng/bench, default structures]
# Forced full render with the engine's `bluemap` CLI. Heap mode leaves stacks at /tmp/eng.heap.txt (aggregated
# on the box: heaptrack's folded export runs to GBs).
set -e
cd ~/bmrs-eng/bench/${2:-structures}
EXE=~/bmrs-eng/target/profiling/bluemap
ARGS="-c config -v 26.3 -r -f"
rm -rf web data/logs
case "$1" in
  plain) flock ~/bench.lock /usr/bin/time -f "user %U wall %e rss %MkB" $EXE $ARGS 2>&1 >/dev/null | tail -1 ;;
  perf) flock ~/bench.lock perf record -F 499 -g --call-graph dwarf,16384 -o /tmp/eng.perf.data $EXE $ARGS >/dev/null 2>&1
        perf script -i /tmp/eng.perf.data 2>/dev/null | ~/.cargo/bin/inferno-collapse-perf --all | ~/.cargo/bin/rustfilt > /tmp/eng.folded
        rm -f /tmp/eng.perf.data; wc -l /tmp/eng.folded ;;
  heap) flock ~/bench.lock heaptrack -o /tmp/eng.heap $EXE $ARGS >/dev/null 2>&1
        heaptrack_print -f /tmp/eng.heap.zst -p 1 -a 0 -T 0 -n 0 -s 0 2>/dev/null | tail -6
        heaptrack_print -f /tmp/eng.heap.zst --flamegraph-cost-type peak -F /tmp/eng.heap.raw >/dev/null 2>&1
        ~/.cargo/bin/rustfilt < /tmp/eng.heap.raw > /tmp/eng.heap.folded; rm -f /tmp/eng.heap.raw ;;
  timeline) flock ~/bench.lock perf record -F 199 -s -g --call-graph dwarf,8192 -o /tmp/eng.tl.data $EXE $ARGS >/dev/null 2>&1
        perf script -i /tmp/eng.tl.data -F comm,tid,time 2>/dev/null > /tmp/eng.tl.txt; rm -f /tmp/eng.tl.data; wc -l /tmp/eng.tl.txt ;;
esac
du -sh web 2>/dev/null || true
