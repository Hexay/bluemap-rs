#!/bin/bash
# usage: bash crates/bm-wasm/bench/native_perf.sh <corpus dir> [perf out dir]: native unpack time (3 runs) and a perf
# profile of the BMQ3 decoder on a corpus from the `corpus` example (Linux; run from the checkout)
set -e
CORPUS=$1; OUT=${2:-$HOME/websrv-prof}
mkdir -p "$OUT"
cargo build -q --profile profiling -p bm-wasm --example unpack_bench
EXE=target/profiling/examples/unpack_bench
for _ in 1 2 3; do "$EXE" "$CORPUS" --reps 5; done
perf record -q -F 2999 -o "$OUT/unpack.data" "$EXE" "$CORPUS" --reps 20 > /dev/null
perf report -i "$OUT/unpack.data" --no-children --sort sym --stdio 2> /dev/null | grep -E '^ +[0-9]' | head -15
