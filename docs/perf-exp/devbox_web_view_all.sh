#!/bin/bash
# usage: devbox_web_view_all.sh <checkout> [runs=5]   under ~/bench.lock, ~70 min
# The three sets of docs/results/devbox-web-java-vs-rs-2026-10-28.md's "One view in the real webapp".
CO=$1; RUNS=${2:-5}
W=$CO/work
HERE=$CO/docs/perf-exp
A=("$CO" "$W/real/j12" "$W/real/d12" "$W/real/d12-opt" "$W/downloads/jdk25/bin/java" "$W/downloads/bluemap-5.28-cli.jar"
  "$HOME/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome" "$RUNS")
echo "=== set 1: SwiftShader draws"
OUT=$HOME/jvr-view/gl bash "$HERE/devbox_web_view.sh" "${A[@]}"
echo "=== set 2: GL draws off"
OUT=$HOME/jvr-view/nodraw VIEW_ARGS=--no-gl-draw bash "$HERE/devbox_web_view.sh" "${A[@]}"
echo "=== set 3: GL draws off, through the MSS 1448 relay"
OUT=$HOME/jvr-view/relay VIEW_ARGS=--no-gl-draw ROWS=j529+rc+ro+rn PROXY=$HERE/mss_relay.py FRONT_PORT=8101 \
  bash "$HERE/devbox_web_view.sh" "${A[@]}"
echo "=== sizes of the view's tiles, Java's render vs ours"
while read -r t; do stat -c %s "$W/real/j12/web/$t.gz" "$W/real/d12/web/$t.gz"; done < "$HOME/jvr-view/gl/view-tiles.txt" \
  | paste - - | awk '{a += $1; b += $2} END {print "j12", a, "d12", b}'
for set in gl nodraw relay; do rm -rf "$HOME/jvr-view/$set"/{j528,j529,rc,ro}; done
