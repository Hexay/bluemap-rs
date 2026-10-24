#!/bin/bash
# usage: testbox_cpu.sh <plain|perf|sched|threads> <fixture dir> [binary=~/bmrs-cpu/target/profiling/bluemap] [hz=199]
# Forced full render (`-c config -v 26.3 -r -f`) of a private fixture copy. perf: on-CPU dwarf stacks for every
# thread, folded to /tmp/cpu.<fx>.folded. sched: sched_switch/wakeup tracepoints (off-CPU, needs paranoid <= -1 or
# CAP_PERFMON) to /tmp/cpu.<fx>.sched. threads: per-thread utime/stime from /proc at exit (no profiler overhead).
set -e
MODE=$1; FX=$2; EXE=${3:-~/bmrs-cpu/target/profiling/bluemap}; HZ=${4:-199}
NAME=$(basename "$FX")
cd "$FX"
ARGS="-c config -v 26.3 -r -f"
rm -rf web data/logs
case "$MODE" in
  plain) /usr/bin/time -f "user %U sys %S wall %e rss %MkB" $EXE $ARGS 2>&1 >/dev/null | tail -1 ;;
  perf) perf record -F "$HZ" -g --call-graph dwarf,16384 -o /tmp/cpu.$NAME.data $EXE $ARGS >/dev/null 2>&1
        perf script -i /tmp/cpu.$NAME.data 2>/dev/null | ~/.cargo/bin/inferno-collapse-perf --all --tid | ~/.cargo/bin/rustfilt > /tmp/cpu.$NAME.folded
        rm -f /tmp/cpu.$NAME.data; wc -l /tmp/cpu.$NAME.folded ;;
  sched) perf record -e sched:sched_switch -e sched:sched_wakeup -o /tmp/cpu.$NAME.sdata $EXE $ARGS >/dev/null 2>&1
         perf script -i /tmp/cpu.$NAME.sdata -F comm,tid,time,event,trace 2>/dev/null > /tmp/cpu.$NAME.sched
         rm -f /tmp/cpu.$NAME.sdata; wc -l /tmp/cpu.$NAME.sched ;;
  threads) $EXE $ARGS >/dev/null 2>&1 & pid=$!
           # snapshot each thread's ticks until exit; the last snapshot per tid wins
           : > /tmp/cpu.$NAME.threads
           while kill -0 $pid 2>/dev/null; do
             for t in /proc/$pid/task/*; do
               [ -r $t/stat ] && echo "$(basename $t) $(tr -d '()' < $t/stat | awk '{print $2, $14, $15}')" >> /tmp/cpu.$NAME.threads
             done 2>/dev/null; sleep 0.2
           done
           awk '{u[$1]=$3; s[$1]=$4; n[$1]=$2} END {for (k in u) c[n[k]]+=u[k]+s[k]; for (k in c) printf "%8.2f s %s\n", c[k]/100, k}' \
             /tmp/cpu.$NAME.threads | sort -rn ;;
esac
du -sh web 2>/dev/null || true
