#!/bin/bash
# usage: shapes_bench.sh <base checkout> <new checkout> [runs=3] [tiles dir]
# Builds compact_bench in both checkouts, then runs them interleaved under ~/bench.lock on every 16th tile of the
# real render (single thread, encode reps only). Prints each run's encode line and blob hash.
set -e
BASE=$1; NEW=$2; RUNS=${3:-3}
TILES=${4:-$HOME/bmrs-real/work/real/rs/web/maps/real/tiles/0}
for co in "$BASE" "$NEW"; do (cd "$co" && ~/.cargo/bin/cargo build -q --release -p bm-format --example compact_bench); done
for i in $(seq "$RUNS"); do
  for co in "$BASE" "$NEW"; do
    echo "== $co"
    flock ~/bench.lock "$co/target/release/examples/compact_bench" "$TILES" --step 16 --reps 5 --encode-reps-only | tail -2
  done
done
