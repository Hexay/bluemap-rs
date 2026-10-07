#!/bin/bash
# usage: prof.sh <plain|perf|heaptrack> — render the structures fixture under the chosen tool, holding the bench lock
set -e
cd ~/bmrs-prof
EXE=target/profiling/examples/render_map
ARGS="work/worlds/structures/world minecraft:overworld work/bluemap/structures/web /tmp/bmrs-out --jar work/client.jar"
rm -rf /tmp/bmrs-out
case "$1" in
  plain) flock ~/bench.lock /usr/bin/time -v $EXE $ARGS 2>&1 | grep -E 'tiles|chunks|resources|meshing|Elapsed|Maximum resident|User time|System time' ;;
  perf) flock ~/bench.lock perf record -F 499 -g --call-graph dwarf,16384 -o /tmp/bmrs.perf.data $EXE $ARGS >/dev/null 2>&1
        perf script -i /tmp/bmrs.perf.data 2>/dev/null | ~/.cargo/bin/inferno-collapse-perf > /tmp/bmrs.folded
        wc -l /tmp/bmrs.folded ;;
  heaptrack) flock ~/bench.lock heaptrack -o /tmp/bmrs.heap $EXE $ARGS >/dev/null 2>&1
        heaptrack_print -f /tmp/bmrs.heap.zst -p 0 -a 1 -T 1 -n 12 -s 12 > /tmp/bmrs.heap.txt 2>&1; tail -12 /tmp/bmrs.heap.txt ;;
esac
