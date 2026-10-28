#!/usr/bin/env bash
# Runs a published image the way upstream's wiki does (MIGRATING.md "CLI / Docker"), as a non-root --user:
# default configs from a bare /app/config, accept-download, render a tiny world, serve with -w, curl, docker stop.
#   [PORT=8100] tools/docker_smoke.sh ghcr.io/hexay/bluemap-rs:latest
# Needs docker, curl, python3 and internet (the client jar comes from Mojang). Fails on the first broken step.
set -euo pipefail
image=$1
tools=$(cd "$(dirname "$0")" && pwd)
dir=$(mktemp -d)
mkdir "$dir"/{config,world,data,web}
user=(--user "$(id -u):$(id -g)")
mounts=(-v "$dir/config:/app/config" -v "$dir/world:/app/world" -v "$dir/data:/app/data" -v "$dir/web:/app/web")
name=bmrs-smoke-$$
port=${PORT:-8100}
trap 'docker rm -f "$name" >/dev/null 2>&1 || true' EXIT
step() { echo "== $*"; }
fail() { echo "FAIL: $*" >&2; docker logs "$name" 2>&1 | tail -n 30 >&2 || true; exit 1; }

docker pull -q "$image"
step "image $(docker image inspect "$image" --format '{{.Os}}/{{.Architecture}}'), host $(uname -m)"
docker run --rm "$image" --version

step "first run: default configs"
docker run --rm "${user[@]}" -v "$dir/config:/app/config" "$image" || true
for f in core.conf webserver.conf maps/overworld.conf; do [ -f "$dir/config/$f" ] || fail "no config/$f"; done
grep -q '^accept-download: false' "$dir/config/core.conf" || fail "core.conf: no accept-download: false"
sed -i 's/^accept-download: false/accept-download: true/' "$dir/config/core.conf"

step "render tiny world (-r)"
python3 "$tools/tiny_world.py" "$dir/world"
docker run --rm "${user[@]}" "${mounts[@]}" "$image" -r || fail "render exited $?"
ls "$dir"/data/minecraft-client-*.jar >/dev/null || fail "client jar not downloaded"

step "serve (-w) and fetch"
docker run -d --name "$name" "${user[@]}" -p "$port:8100" "${mounts[@]}" "$image" -w >/dev/null
got=$dir/got
fetch() { curl -fsS --retry 30 --retry-delay 1 --retry-all-errors -o "$got" "http://127.0.0.1:$port/$1" || fail "GET $1"; }
fetch index.html; grep -qi '<html' "$got" || fail "index.html is no webapp"
fetch maps/overworld/settings.json; grep -q '"hires"' "$got" || fail "map settings.json has no hires"
fetch maps/overworld/tiles/1/x0/z0.png; [ "$(head -c 4 "$got" | tail -c 3)" = PNG ] || fail "lowres tile is no PNG"
# an inner tile: edge tiles lack generated neighbour chunks and are skipped
fetch maps/overworld/tiles/0/x1/z1.prbm; [ -s "$got" ] || fail "hires tile is empty"; echo "hires tile $(wc -c < "$got") B"

step "docker stop"
start=$(date +%s)
docker stop -t 20 "$name" >/dev/null
code=$(docker inspect "$name" --format '{{.State.ExitCode}}')
echo "stopped in $(( $(date +%s) - start ))s, exit $code"
[ "$code" = 0 ] || fail "exit code $code after SIGTERM"
[ -z "$(find "$dir/web" "$dir/data" ! -user "$(id -u)" -print -quit)" ] || fail "files not owned by --user"
echo "PASS $image"
