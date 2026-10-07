#!/bin/bash
# usage: busy_timeline.sh <fixture dir> <binary> — forced render; prints busy cores per 100 ms from /proc (no profiler
# overhead, unlike perf -s with call graphs) plus the process's stage log lines with their offsets.
set -e
cd "$1"
rm -rf web data/logs
flock ~/bench.lock bash -c '
  start=$(date +%s.%N); "'"$2"'" -c config -v 26.3 -r -f > /tmp/busy.log 2>&1 & pid=$!
  hz=$(getconf CLK_TCK); prev=0; t=0
  while kill -0 $pid 2>/dev/null; do
    ticks=$(awk "{print \$14+\$15}" /proc/$pid/stat 2>/dev/null || echo $prev)
    [ $t -gt 0 ] && printf "%5.1fs %5.1f\n" $(echo "$t/10" | bc -l) $(echo "($ticks-$prev)/$hz*10" | bc -l)
    prev=$ticks; t=$((t+1)); sleep 0.1
  done'
grep -nE "Loading|Loaded|Rendering|render|Saving|Done|took" /tmp/busy.log | head -20
