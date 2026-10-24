#!/bin/bash
# docs/19: BMQ3 codec size + single-thread time on testbox's real render (every 16th tile), then zstd level times.
# Usage (from the checkout root): bash docs/storage-exp/r2_bench.sh [tiles dir] [baseline shapes.rs]
# With a baseline shapes.rs, the same runs follow with it swapped in (and the tree's version restored after).
set -e
TILES=${1:-$HOME/bmrs-real/work/real/rs/web/maps/real/tiles/0}
SHAPES=crates/bm-format/src/compact/shapes.rs
run() {
    cargo build -q --release -p bm-format --examples
    for i in 1 2; do target/release/examples/compact_bench "$TILES" --step 16 --reps 3; done
    target/release/examples/compact_exp "$TILES" --step 16 --only z9 --time
}
echo "== tree"; run
if [ -n "$2" ]; then
    cp "$SHAPES" /tmp/shapes_tree.rs
    trap 'cp /tmp/shapes_tree.rs "$SHAPES"' EXIT
    cp "$2" "$SHAPES"
    echo "== baseline"; run
fi
