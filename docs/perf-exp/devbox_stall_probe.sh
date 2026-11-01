#!/bin/bash
# usage: devbox_stall_probe.sh <checkout> <devbox_web_view.sh's $OUT, with its roots> <java> <bluemap cli jar>
# stall_probe.py against Java and ours at several TCP MSS values. Run under ~/bench.lock.
CO=$1; O=$2; JAVA=$3; JAR=$4
for srv in java rs; do
  if [ $srv = java ]; then
    (cd "$O/j528" && exec "$JAVA" -jar "$JAR" -w) > /dev/null 2>&1 &
  else
    (cd "$O/rc" && exec "$CO/target/release/bluemap" -w) > /dev/null 2>&1 &
  fi
  PID=$!
  until (exec 3<>/dev/tcp/127.0.0.1/8100) 2> /dev/null; do sleep 0.1; done
  "$CO/target/profiling/examples/load_bench" 127.0.0.1:8100 "$O/warm.txt" --conns 32 --secs 3 \
    --header "Accept-Encoding: gzip, deflate, br, zstd" > /dev/null
  for mss in 0 8960 1448 1200; do
    a=(--mss "$mss"); [ "$mss" = 0 ] && a=()
    echo "$srv $(python3 "$CO/docs/perf-exp/stall_probe.py" 127.0.0.1:8100 "$O/warm.txt" "${a[@]}" --n 200)"
  done
  kill $PID; wait $PID 2> /dev/null
done
