#!/bin/sh
# Regenerates bm-java/bm-math/bm-resources/bm-map test data (and bm-java's gray LUTs) from the real Java code.
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
# model rotations: keep java_ref.rs's 12-line header (struct definition), regenerate the ROTATIONS table
"$J/javac" -nowarn -d "$OUT" -sourcepath "$FLOWMATH;$BLUEMAP/core/src/main/java" "$ROOT/tools/javaref/ModelRef.java"
MODEL_REF="$ROOT/crates/bm-resources/src/model/java_ref.rs"
{ head -12 "$MODEL_REF"; "$J/java" -cp "$OUT" ModelRef; } > "$OUT/java_ref.rs" && mv "$OUT/java_ref.rs" "$MODEL_REF"
# Double/Float.toString and (Concurrent)HashMap order (plain JDK)
"$J/javac" -nowarn -d "$OUT" "$ROOT/tools/javaref/JdkRef.java"
"$J/java" -cp "$OUT" JdkRef "$ROOT/crates/bm-java/tests/data"
# PNG corpus (every colour type / bit depth) through ImageIO.read + getRGB + ImageIO.write, and the gray LUTs
"$J/javac" -nowarn -d "$OUT" "$ROOT/tools/javaref/PngRef.java"
"$J/java" -cp "$OUT" PngRef "$ROOT/crates/bm-resources/tests/data" "$ROOT/crates/bm-java/src/png"
# render masks: the shipped 5.28 classes (sources need lombok); own out dir so no source-compiled class shadows the jar
CLI=$ROOT/work/downloads/bluemap-5.28-cli.jar
mkdir -p "$OUT/mask" "$ROOT/crates/bm-map/tests/data"
"$J/javac" -nowarn -d "$OUT/mask" -cp "$CLI" "$ROOT/tools/javaref/MaskRef.java"
"$J/java" -cp "$OUT/mask;$CLI" MaskRef "$ROOT/crates/bm-map/tests/data/mask.rs"
