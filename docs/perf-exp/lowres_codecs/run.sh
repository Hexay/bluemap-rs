#!/bin/bash
# usage: run.sh <checkout> <mode>   (modes below; timed takes ~/bench.lock itself)
# Fixtures read-only. Output under /tmp/lowres-exp. Results: docs/results/lowres-codecs-2026-11-01.md
set -e
CO=$1; MODE=$2
D12=$HOME/bmrs-devbench/work/real/d12/web/maps/real/tiles
K8=$HOME/bmrs-devbench/work/scale/8-optimized/web/maps/real/tiles
CHROME=$HOME/.cache/ms-playwright/chromium-1248/chrome-linux64/chrome
OUT=/tmp/lowres-exp; mkdir -p $OUT
export CARGO_TARGET_DIR=/tmp/lowres-target
cargo build --release -q --manifest-path "$CO/docs/perf-exp/lowres_codecs/Cargo.toml"
BIN=$CARGO_TARGET_DIR/release/lowres_codecs
AGG="python3 $CO/docs/perf-exp/lowres_codecs/agg.py"
case $MODE in
  # sizes + verification on every tile (slow candidates on a size-stratified subset); several may run at once
  sizes-d12) $BIN "$D12" $OUT/d12.tsv --slow-n 8; $AGG $OUT/d12.tsv; echo VIEW; $AGG $OUT/d12.tsv --view ;;
  sizes-k8) $BIN "$K8" $OUT/k8.tsv --n 96,64,16 --slow-n 6; $AGG $OUT/k8.tsv ;;
  # the candidates the subsets above flagged, on every tile
  extra-d12) $BIN "$D12" $OUT/d12x.tsv --slow-n 9999 --only "webp z7,webp z8,webp z9,zstd19 raw,zstd19 pixels,palette"; $AGG $OUT/d12x.tsv; echo VIEW; $AGG $OUT/d12x.tsv --view ;;
  extra-k8) $BIN "$K8" $OUT/k8x.tsv --n 96,64,16 --slow-n 9999 --only "webp z7,webp z8,webp z9,zstd19 raw,zstd19 pixels,palette"; $AGG $OUT/k8x.tsv ;;
  # one thread, nothing else of ours running: a smaller subset, slow ones capped
  timed) flock "$HOME/bench.lock" $BIN "$K8" $OUT/timed.tsv --n 24,16,8 --slow-n 3; $AGG $OUT/timed.tsv
    $BIN "$D12" $OUT/product.tsv --only "product"; $AGG $OUT/product.tsv ;;
  browser) rm -rf $OUT/emit; $BIN "$D12" /dev/null --emit $OUT/emit; node "$CO/docs/perf-exp/lowres_codecs/browser_check.mjs" $OUT/emit "$CHROME" ;;
esac
