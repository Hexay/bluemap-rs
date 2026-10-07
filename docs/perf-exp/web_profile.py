"""usage: web_profile.py <webroot> <out.json.gz> [--secs 40] [--analyze-only] [--top 40]

Records serve_bench under samply (elevated: one UAC prompt) while web_bench.py drives a scenario mix, then prints
self/inclusive hot functions. Reuses bluemap_reverse/tools/profile.py (run_elevated, analyze, MSVC-map symbols).
Build first (map file = symbols samply can't get from the Rust PDB):
  cargo rustc -p bm-web --profile profiling --example serve_bench -- -C link-arg=/MAP:<repo>/target/profiling/serve_bench.map
"""
import argparse, subprocess, sys, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT.parent / "bluemap_reverse" / "tools"))
import profile as bmr_profile  # noqa: E402

EXE = ROOT / "target" / "profiling" / "examples" / "serve_bench.exe"
MAP = ROOT / "target" / "profiling" / "serve_bench.map"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("webroot")
    ap.add_argument("out", type=Path)
    ap.add_argument("--secs", type=int, default=40)
    ap.add_argument("--port", type=int, default=8124)
    ap.add_argument("--only", default="hires_gzip,hires_identity,lowres_png,map_settings_gzip,textures_identity,static_js_1.2MB,static_small,page_view_mix")
    ap.add_argument("--top", type=int, default=40)
    ap.add_argument("--analyze-only", action="store_true")
    a = ap.parse_args()
    if not a.analyze_only:
        args = ["record", "--save-only", "--unstable-presymbolicate", "--rate", "2000", "-o", str(a.out), "--",
                str(EXE), str(a.webroot), "--port", str(a.port), "--exit-after", str(a.secs)]
        rec = subprocess.Popen([sys.executable, "-c",
                                "import sys; sys.path.insert(0, sys.argv[1]); import profile as p; from pathlib import Path;"
                                "raise SystemExit(p.run_elevated(Path(sys.argv[2]), sys.argv[4:], Path(sys.argv[3])))",
                                str(ROOT.parent / "bluemap_reverse" / "tools"), str(bmr_profile.SAMPLY), str(ROOT), *args])
        time.sleep(6)
        per = max(2, (a.secs - 10) // len(a.only.split(",")))
        subprocess.run([sys.executable, str(Path(__file__).with_name("web_bench.py")), str(a.webroot), "--attach",
                        "--port", str(a.port), "--secs", str(per), "--only", a.only, "--sse", "0"])
        if rec.wait() or not a.out.exists():
            raise SystemExit("samply failed")
    bmr_profile.analyze(a.out, a.top, EXE.name, MAP if MAP.exists() else None)


if __name__ == "__main__":
    main()
