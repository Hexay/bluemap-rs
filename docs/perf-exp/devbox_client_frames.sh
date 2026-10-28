#!/bin/bash
# usage: [BASE=<commit>] devbox_client_frames.sh <checkout> <optimized webroot> <chrome> [runs=3]
# Browsers without zstd (docs/18): the shipped client script in headless Chrome against `serve_bench --optimized`,
# Accept-Encoding as Chrome sends it and without zstd, then time to a rendered view (crates/bm-wasm/bench/view.mjs)
# for the script of the parent commit (bodies only: no zstd means a server-side gzip transcode) against this one.
set -e
CO=$1; OPT=$2; CHROME=$3; RUNS=${4:-3}
PORT=8133
NO_ZSTD="gzip, deflate, br"
(cd "$CO" && ~/.cargo/bin/cargo build -q --profile profiling -p bm-web --example serve_bench 2>&1 | tail -3)
T=$(mktemp -d)
# a sandbox-less Chrome: the box has no unprivileged user namespaces
printf '#!/bin/sh\nexec "%s" --no-sandbox "$@"\n' "$CHROME" > "$T/chrome"; chmod +x "$T/chrome"
cp "$CO/crates/bm-web/client/unpack.js" "$T/new.js"; cp "$CO/crates/bm-web/client/unpack.wasm" "$T/new.wasm"
# BASE: the commit whose script to compare with (default HEAD~1)
git -C "$CO" show "${BASE:-HEAD~1}:crates/bm-web/client/unpack.js" > "$T/old.js"
git -C "$CO" show "${BASE:-HEAD~1}:crates/bm-web/client/unpack.wasm" > "$T/old.wasm"
"$CO/target/profiling/examples/serve_bench" "$OPT" --port $PORT --optimized --server-unpack limited > /dev/null 2>&1 &
SERVER=$!
trap 'kill $SERVER 2> /dev/null' EXIT
until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2> /dev/null; do sleep 0.1; done
MAP=$(ls "$OPT/maps" | grep -v '\.txt$' | head -1)
URL="http://127.0.0.1:$PORT/#$MAP:0:70:0:60:0:0.9:0:0:perspective"
echo "== browser check, Chrome's Accept-Encoding"
node "$CO/tools/check_client_unpack_browser.mjs" "$URL" "$T/chrome" || echo "BROWSER CHECK FAILED"
echo "== browser check, $NO_ZSTD"
node "$CO/tools/check_client_unpack_browser.mjs" "$URL" "$T/chrome" --accept-encoding "$NO_ZSTD" || echo "BROWSER CHECK FAILED"
echo "== time to view"
for ae in "" "$NO_ZSTD"; do
  for s in old new; do
    echo "-- $s script, Accept-Encoding: ${ae:-as Chrome sends it}"
    node "$CO/crates/bm-wasm/bench/view.mjs" "$URL" "$T/chrome" --script "$T/$s.js" --wasm "$T/$s.wasm" --runs "$RUNS" \
      ${ae:+--accept-encoding "$ae"} | python3 -c "import json,sys; r=json.load(sys.stdin); print(r['median'], [x['loaded'] for x in r['runs']], 'errors', sum(x['errors'] for x in r['runs']))"
  done
done
echo "-- no script, $NO_ZSTD"
node "$CO/crates/bm-wasm/bench/view.mjs" "$URL" "$T/chrome" --no-script --runs "$RUNS" --accept-encoding "$NO_ZSTD" \
  | python3 -c "import json,sys; r=json.load(sys.stdin); print(r['median'], [x['loaded'] for x in r['runs']])"
rm -rf "$T"
