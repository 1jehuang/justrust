#!/usr/bin/env python3
"""Hosted type-error loop: wall time of `justrust check --lib` after an
agent-style E0308 edit, with the hosted early-exit default vs opted out
(JUSTRUST_EARLY_EXIT=0). Run from a workspace whose hosted early daemon is
warm. Usage: hosted_loop.py <justrust-binary> <rounds>"""
import json, os, random, subprocess, sys, time

exe, rounds = sys.argv[1], int(sys.argv[2])
files = ["src/procfs.rs", "src/paths.rs", "src/slots.rs", "src/update.rs", "src/early.rs"]
rng = random.Random(1)
out = {"rounds": []}


def check(env_extra):
    env = dict(os.environ, **env_extra)
    t = time.time()
    p = subprocess.run([exe, "check", "--lib"], env=env, capture_output=True, text=True)
    wall = time.time() - t
    text = p.stdout + p.stderr
    return wall, p.returncode, "returned early" in text


for r in range(rounds):
    for mode, extra in (("default", {}), ("opt_out", {"JUSTRUST_EARLY_EXIT": "0"})):
        f = rng.choice(files)
        src = open(f).read()
        bad = src + f"\npub fn __bench_err_{r}_{mode}() -> u32 {{ \"not a number\" }}\n"
        open(f, "w").write(bad)
        try:
            wall, code, early = check(extra)
        finally:
            open(f, "w").write(src)
        ok_wall, ok_code, _ = check(extra)
        row = dict(round=r, mode=mode, file=f, err_wall=round(wall, 2), err_code=code,
                   early=early, fix_wall=round(ok_wall, 2), fix_code=ok_code)
        print(json.dumps(row), flush=True)
        out["rounds"].append(row)

for mode in ("default", "opt_out"):
    rows = [x for x in out["rounds"] if x["mode"] == mode]
    errs = sorted(x["err_wall"] for x in rows)
    print(mode, "error-run median", errs[len(errs) // 2], "s; early", sum(x["early"] for x in rows), "/", len(rows))
