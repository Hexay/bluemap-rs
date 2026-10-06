#!/bin/sh
# Regenerates bm-java/bm-math test data from the real Java code.
# Usage: sh tools/javaref/run.sh <BlueMap checkout> <flow-math 1.0.3 sources (unpacked -sources.jar)>
# flow-math sources: https://repo1.maven.org/maven2/com/flowpowered/flow-math/1.0.3/flow-math-1.0.3-sources.jar
set -e
BLUEMAP=${1:?BlueMap checkout}
FLOWMATH=${2:?flow-math sources}
ROOT=$(dirname "$0")/../..
J=$ROOT/work/downloads/jdk25/bin
OUT=$ROOT/work/javaref
mkdir -p "$OUT" "$ROOT/crates/bm-java/tests/data" "$ROOT/crates/bm-math/tests/data"
"$J/javac" -nowarn -d "$OUT" -sourcepath "$FLOWMATH;$BLUEMAP/core/src/main/java" "$ROOT/tools/javaref/MathRef.java"
"$J/java" -cp "$OUT" MathRef "$ROOT/crates/bm-java/tests/data" "$ROOT/crates/bm-math/tests/data"
mkdir -p "$ROOT/crates/bm-resources/tests/data"
"$J/javac" -nowarn -d "$OUT" -sourcepath "$FLOWMATH;$BLUEMAP/core/src/main/java" "$ROOT/tools/javaref/ColorRef.java"
"$J/java" -cp "$OUT" ColorRef "$ROOT/crates/bm-resources/tests/data/color.rs"
