#!/usr/bin/env python3
"""The tests of a project that fail in some runs and not in others, under plain cargo and
under `cargo ra-jit`: the suite is run N times by each, and each test is counted by the
runs it failed in. A test that fails as often under both is the project's own.

  python3 flakes.py <checkout> [runs] [cargo|ra-jit]

With `FLAKES_KEEP` set to a directory, what each run printed is kept there.
"""
import collections
import os
import re
import subprocess
import sys

FAILED = re.compile(r"^test (\S+) \.\.\. FAILED", re.M)


def runs_of(checkout, count, ra_jit, env):
    """The tests each of `count` runs failed, and who built each run."""
    argv = ["cargo", *(["ra-jit"] if ra_jit else []), "test", "--no-fail-fast"]
    failed, built = collections.Counter(), collections.Counter()
    keep = os.environ.get("FLAKES_KEEP")
    for run in range(count):
        ran = subprocess.run(argv, cwd=checkout, env=env, capture_output=True, text=True, stdin=subprocess.DEVNULL)
        failed.update(set(FAILED.findall(ran.stdout)))
        if keep:
            with open(os.path.join(keep, f"{'ra-jit' if ra_jit else 'cargo'}-{run}.log"), "w") as kept:
                kept.write(ran.stdout + ran.stderr)
        built["daemon" if "[blaze]" in ran.stderr else "cargo"] += 1
    return failed, built


def main():
    checkout, count = sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 100
    sides = sys.argv[3:] or ["cargo", "ra-jit"]
    env = dict(os.environ)
    if "RA_JIT_BIN" in env:
        env["PATH"] = os.path.dirname(env["RA_JIT_BIN"]) + os.pathsep + env["PATH"]
    for side in sides:
        failed, built = runs_of(checkout, count, side == "ra-jit", env)
        print(f"{side}: {count} runs, built by {dict(built)}; runs each test failed in: {dict(failed)}", flush=True)


if __name__ == "__main__":
    main()
