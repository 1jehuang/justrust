#!/usr/bin/env python3
"""wasm32.py: what CI checks of wasm32 on every merge (the train's `wasm32` job), and what a
lane runs in its VM before landing a change to the wasm lowering.

Each check is a harness run with `RA_JIT_TARGET` set, in a process of its own, so that the
target reaches everything the harness imports before it reads it:

  - corpus: `difftest.py corpus` for wasm32-wasip1 under wasmtime, against cargo's build;
  - stress: `stress.py` for wasm32-wasip1, the daemon's edits on a small std program;
  - allrajit: `allrajit_wasm.py`, programs and libraries with nothing of rustc's (wasip1
    programs, a wasm32-unknown-unknown cdylib and a `#![no_std]` simd128 guest), their target
    their own.

Needs the targets (`rustup target add wasm32-wasip1 wasm32-unknown-unknown`) and wasmtime on
PATH, which the flake's `wasm` shell has (`nix develop .#wasm`). What each check printed last
is shown, and the run fails when any did.

    python3 wasm32.py               # every check
    python3 wasm32.py corpus        # the ones named

Python 3, stdlib only.
"""
import os
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))

# (name, target, the harness's arguments, a timeout in seconds)
CHECKS = [
    ("corpus", "wasm32-wasip1", ["difftest.py", "corpus", "--bin", "runner", "--crate", "cases"], 1800),
    ("stress", "wasm32-wasip1", ["stress.py"], 1800),
    ("allrajit", None, ["allrajit_wasm.py"], 1800),
]


def run(name, target, args, timeout):
    env = {**os.environ, "RUST_BACKTRACE": "0"}
    if target:
        env["RA_JIT_TARGET"] = target
    start = time.monotonic()
    try:
        ran = subprocess.run([sys.executable, *args], cwd=HERE, env=env, stdin=subprocess.DEVNULL,
                             capture_output=True, text=True, timeout=timeout)
        status, said = ran.returncode, ran.stdout + ran.stderr
    except subprocess.TimeoutExpired as e:
        status, said = None, f"{e.stdout or ''}{e.stderr or ''}\nstill running after {timeout} s"
    took = time.monotonic() - start
    verdict = "ok" if status == 0 else "FAIL"
    print(f"{verdict:4}  {name} ({target or 'its own targets'}, {took:.0f} s)")
    if status != 0:
        print(said[-6000:])
    else:
        print("\n".join("      " + line for line in said.strip().splitlines()[-3:]))
    return status == 0


def main():
    asked = sys.argv[1:]
    unknown = [it for it in asked if it not in {name for name, *_ in CHECKS}]
    if unknown:
        raise SystemExit(f"no such check: {', '.join(unknown)}")
    if not shutil.which("wasmtime"):
        raise SystemExit("wasmtime is not on PATH: run in `nix develop .#wasm`")
    results = [run(*check) for check in CHECKS if not asked or check[0] in asked]
    print(f"wasm32: {sum(results)} of {len(results)} checks passed")
    sys.exit(not all(results))


if __name__ == "__main__":
    main()
