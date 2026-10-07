#!/bin/bash
# usage: eng.sh <plain|perf|heaptrack|offcpu> — forced full render of bench/structures with the engine's `bluemap` CLI
set -e
cd ~/bmrs-eng/bench/structures
EXE=~/bmrs-eng/target/profiling/bluemap
ARGS="-c config -v 26.3 -r -f"
rm -rf web data/logs
case "$1" in
  plain) flock ~/bench.lock /usr/bin/time -v $EXE $ARGS 2>&1 | grep -E 'Elapsed|Maximum resident|User time|System time|Voluntary|Involuntary|File system outputs' ;;
  perf) flock ~/bench.lock perf record -F 499 -g --call-graph dwarf,16384 -o /tmp/eng.perf.data $EXE $ARGS >/dev/null 2>&1
        perf script -i /tmp/eng.perf.data 2>/dev/null | ~/.cargo/bin/inferno-collapse-perf --all | ~/.cargo/bin/rustfilt > /tmp/eng.folded
        wc -l /tmp/eng.folded ;;
  timeline) flock ~/bench.lock perf record -F 199 -s -g --call-graph dwarf,8192 -o /tmp/eng.tl.data $EXE $ARGS >/dev/null 2>&1
        perf script -i /tmp/eng.tl.data -F comm,tid,time 2>/dev/null > /tmp/eng.tl.txt; wc -l /tmp/eng.tl.txt ;;
esac
du -sh web 2>/dev/null || true
