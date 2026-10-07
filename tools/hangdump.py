"""Evidence from a hung process before it is killed: a minidump (Windows, no admin needed for own processes) and,
when `minidump-stackwalk` and `dump_syms` are installed (`cargo install minidump-stackwalk dump_syms`), its
symbolized thread stacks.

Usage: py -3 tools/hangdump.py <pid> [exe]  |  py -3 tools/hangdump.py --stacks <dmp> [exe]
The accept scripts call `dump` on every command timeout.
"""
import ctypes
import shutil
import subprocess
import sys
from pathlib import Path

from paths import ROOT, WORK

HANGS = WORK / "hangs"
EXE = ROOT / "target" / "release" / "bluemap.exe"
# ThreadInfo | DataSegs | IndirectlyReferencedMemory: enough for stack walking without a full-memory dump
DUMP_FLAGS = 0x1000 | 0x1 | 0x40


def write_minidump(pid: int, out: Path) -> bool:
    if sys.platform != "win32":
        return False
    from ctypes import wintypes
    k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    dbghelp = ctypes.WinDLL("dbghelp", use_last_error=True)
    k32.OpenProcess.restype = wintypes.HANDLE
    k32.CreateFileW.restype = wintypes.HANDLE
    process = k32.OpenProcess(0x0400 | 0x0010 | 0x0040, False, pid)  # QUERY_INFORMATION | VM_READ | DUP_HANDLE
    if not process:
        return False
    out.parent.mkdir(parents=True, exist_ok=True)
    file = k32.CreateFileW(str(out), 0x40000000, 0, None, 2, 0x80, None)  # GENERIC_WRITE, CREATE_ALWAYS
    ok = dbghelp.MiniDumpWriteDump(process, pid, file, DUMP_FLAGS, None, None, None)
    k32.CloseHandle(file)
    k32.CloseHandle(process)
    return bool(ok)


def stacks(dmp: Path, exe: Path) -> str:
    """Thread names and the bluemap frames of each thread."""
    walk, syms = shutil.which("minidump-stackwalk"), shutil.which("dump_syms")
    pdb = exe.with_suffix(".pdb")
    if not (walk and syms and pdb.is_file()):
        return f"(stacks: install minidump-stackwalk and dump_syms, then run `py -3 tools/hangdump.py --stacks {dmp}`)"
    sym_dir = dmp.parent / "syms"
    subprocess.run([syms, "-s", str(sym_dir), str(pdb)], capture_output=True)
    out = subprocess.run([walk, "--symbols-path", str(sym_dir), str(dmp)], capture_output=True, text=True).stdout
    keep = [line for line in out.splitlines() if line.startswith("Thread ") or (".exe!" in line and "]" in line)]
    return "\n".join(keep)


def dump(pid: int, name: str, exe: Path) -> str:
    """Minidump of `pid` under work/hangs/ plus its stacks, as text to print; never raises."""
    try:
        dmp = HANGS / f"{name}-{pid}.dmp"
        if not write_minidump(pid, dmp):
            return f"(no minidump of {pid})"
        return f"minidump: {dmp}\n{stacks(dmp, exe)}"
    except Exception as e:  # evidence gathering must not mask the hang itself
        return f"(hang dump failed: {e})"


if __name__ == "__main__":
    if sys.argv[1] == "--stacks":
        print(stacks(Path(sys.argv[2]), Path(sys.argv[3]) if len(sys.argv) > 3 else EXE))
    else:
        print(dump(int(sys.argv[1]), "manual", Path(sys.argv[2]) if len(sys.argv) > 2 else EXE))
