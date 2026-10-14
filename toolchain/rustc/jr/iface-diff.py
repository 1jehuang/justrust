#!/usr/bin/env python3
"""Rerun a recorded rustc command for one crate with a different rustc, twice,
with a body edit in between, dumping the interface hash stream each time, and
report the first difference.

Usage: jr/iface-diff.py CMDFILE RUSTC WORKDIR FILE ANCHOR
CMDFILE holds the `Running` line cargo printed (shell quoted). The command is
run from the snapshot root's crate cwd (the `cwd` of the cargo invocation),
with outputs redirected to WORKDIR/o1, WORKDIR/o2.
"""
import os, shlex, shutil, subprocess, sys
from pathlib import Path

cmdfile, rustc, work, file, anchor, cwd = sys.argv[1:7]
work = Path(work)
work.mkdir(parents=True, exist_ok=True)
argv = shlex.split(Path(cmdfile).read_text().strip())
# drop wrappers: find the shim/rustc and replace with ours
i = next(k for k, a in enumerate(argv) if a.endswith("/rustc"))
args = argv[i + 1:]
def set_arg(args, flag, val):
    out = []
    skip = False
    for k, a in enumerate(args):
        if skip:
            skip = False
            continue
        if a == flag:
            skip = True
            continue
        if a.startswith(flag + "="):
            continue
        out.append(a)
    return out + [flag, val]
cleaned = []
k = 0
while k < len(args):
    if args[k] == "-C" and k + 1 < len(args) and args[k + 1].startswith("incremental="):
        k += 2
        continue
    cleaned.append(args[k])
    k += 1
args = cleaned
src = Path(cwd) / file
orig = src.read_text()
assert orig.count(anchor) == 1, "anchor"
hashes = []
try:
    for v, val in zip((1, 2), os.environ.get("VALS", "1 2").split()):
        out = work / f"o{v}"
        out.mkdir(exist_ok=True)
        a = set_arg(args, "--out-dir", str(out))
        if os.environ.get("INCR"):
            a += ["-C", f"incremental={work / 'incr'}"]
        src.write_text(orig.replace(anchor, anchor + f"        std::hint::black_box({val}u64);\n", 1))
        env = dict(os.environ, JUSTRUST_RUSTC_IFACE_HASH="1",
                   JUSTRUST_RUSTC_IFACE_DUMP=str(work / f"dump{v}"), RUSTC_BOOTSTRAP="1")
        r = subprocess.run([rustc] + a, cwd=cwd, env=env, capture_output=True, text=True)
        (work / f"stderr{v}").write_text(r.stderr)  # stderr_dump
        if r.returncode:
            print(r.stderr[-3000:])
            sys.exit(1)
        side = list(out.glob("*.rmeta.jriface"))
        hashes.append(side[0].read_text().strip() if side else None)
finally:
    src.write_text(orig)
print("hashes:", hashes)
d1 = (work / "dump1").read_bytes()
d2 = (work / "dump2").read_bytes()
print("dump sizes:", len(d1), len(d2))
n = next((k for k in range(min(len(d1), len(d2))) if d1[k] != d2[k]), None)
if n is None:
    print("streams equal up to the shorter length")
else:
    print("first difference at", n)
    print("1:", d1[max(0, n - 64):n + 64])
    print("2:", d2[max(0, n - 64):n + 64])
