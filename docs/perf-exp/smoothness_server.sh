#!/bin/bash
# usage: smoothness_server.sh prepare | start <j529|rc|ro|rn> | stop
# The devbox side of crates/bm-wasm/bench/smoothness.mjs: the roots of devbox_web_load_dump.sh under $OUT. Every row-run
# restarts the server and warms it 3 s at 32 conns on tiles far from the bench's camera path (JIT, not the tiles).
# The caller holds ~/bench.lock.
set -e
OUT=${OUT:-$HOME/viewtrace-load}
CO=${CO:-$HOME/bmrs-devbench}
JAVA=$CO/work/downloads/jdk25/bin/java
JAR=$CO/work/downloads/bluemap-5.28-cli.jar
LOAD=$CO/target/profiling/examples/load_bench
PORT=8100
PIDFILE=$OUT/smoothness.pid
WARM=$OUT/smoothness-warm.txt

stop() {
  [ -f "$PIDFILE" ] && kill "$(cat "$PIDFILE")" 2> /dev/null || true
  rm -f "$PIDFILE"
  local i=0
  while (exec 3<> /dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do
    sleep 0.1; i=$((i + 1))
    [ $i -lt 100 ] || { echo "port $PORT still busy"; exit 1; }
  done
}

case $1 in
  prepare)
    "$OUT/bluemap" --version | tail -1
    "$JAVA" -version 2>&1 | head -1
    stop
    # tile x below -30 is 960 blocks from the path; hard-linked j529/rc share these inodes
    (cd "$OUT/rc/web" && find maps/real/tiles/0 -name '*.prbm.gz' | sed 's/\.gz$//') | python3 -c "
import random, re, sys
far = [l.strip() for l in sys.stdin if int(re.search(r'/x(-?[\d/]+)/z', l).group(1).replace('/', '')) < -30]
random.Random(1).shuffle(far)
print('\n'.join(far[:4000]))" > "$WARM"
    echo "warm list: $(wc -l < "$WARM") tiles"
    find "$OUT"/{j529,rc,ro}/web -type f -exec cat {} + > /dev/null
    ;;
  start)
    row=$2
    stop
    root=$row cmd=("$OUT/bluemap" -w)
    case $row in
      j529) cmd=("$JAVA" -jar "$JAR" -w) ;;
      rn) root=ro ;;
    esac
    cd "$OUT/$root"
    nohup "${cmd[@]}" > "$OUT/smoothness-$row.log" 2>&1 < /dev/null &
    echo $! > "$PIDFILE"
    i=0
    until (exec 3<> /dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do
      sleep 0.1; i=$((i + 1))
      [ $i -lt 600 ] || { tail "$OUT/smoothness-$row.log"; exit 1; }
    done
    hdr=(--header "Accept-Encoding: gzip, deflate, br, zstd")
    [ "$row" = ro ] && hdr+=(--header "Accept: application/vnd.bluemap.bmq3, */*")
    "$LOAD" 127.0.0.1:$PORT "$WARM" --conns 32 --secs 3 "${hdr[@]}" > /dev/null
    sleep 1
    cut -d' ' -f1 /proc/loadavg
    ;;
  stop) stop ;;
  *) echo "usage: $0 prepare | start <row> | stop"; exit 2 ;;
esac
