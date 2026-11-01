#!/bin/bash
# usage: devbox_client_checks.sh <checkout> <optimized root with web/ and config/> <compat webroot> [fresh only: fresh] [tuning json]
# The client script's end-to-end checks against a build's own embedded script. Takes port 8100: run under ~/bench.lock.
CO=$1; R=$2; COMPAT=$3; ONLY=$4; TUNE=$5
CHROME=$HOME/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome
W=$(mktemp -d); printf '#!/bin/sh\nexec "%s" --no-sandbox "$@"\n' "$CHROME" > "$W/chrome"; chmod +x "$W/chrome"
(cd "$R" && exec "$CO/target/release/bluemap" -w) > "$W/server.log" 2>&1 &
PID=$!
trap 'kill $PID; rm -rf "$W"' EXIT
for i in $(seq 1 300); do (exec 3<>/dev/tcp/127.0.0.1/8100) 2> /dev/null && break; sleep 0.1; done
code=0
if [ "$ONLY" != fresh ]; then
  timeout 300 node "$CO/tools/check_client_unpack.mjs" http://127.0.0.1:8100 "$COMPAT" 300 || code=1
  timeout 120 node "$CO/tools/check_client_unpack_browser.mjs" "http://127.0.0.1:8100/#real:0:70:0:60:0:0.9:0:0:perspective" "$W/chrome" || code=1
  timeout 120 node "$CO/tools/check_client_unpack_browser.mjs" "http://127.0.0.1:8100/#real:0:70:0:60:0:0.9:0:0:perspective" "$W/chrome" --accept-encoding "gzip,deflate,br" || code=1
fi
timeout 300 node "$CO/tools/check_client_fresh.mjs" http://127.0.0.1:8100 real "$W/chrome" ${TUNE:+--tune "$TUNE"} || code=1
exit $code
