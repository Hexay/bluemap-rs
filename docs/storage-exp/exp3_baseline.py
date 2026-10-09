"""Shipped BMQ2 on a corpus: per-stream breakdown, then whole-body codec sweep with timings.
Usage: exp3_baseline.py [real|fx] [step]"""
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))  # -I drops the script dir
import exp3_lib as lib

if __name__ == "__main__":
    corpus = sys.argv[1] if len(sys.argv) > 1 else "real"
    step = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    lib.run("exp3_lib", ["baseline"], corpus, step, detail=["baseline"])
    lib.sweep("exp3_lib", "baseline", corpus, step * 8)
