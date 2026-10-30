#!/usr/bin/env python3
"""The doctests of real crates, run by `cargo test --doc` and by `cargo ra-jit test --doc`:
the same tests must end the same way and print the same, in ra-jit's first run and in its
second, which reuses the programs of the first. Each crate's line says how many examples
ra-jit built and how many it did not: those fail, saying why, and are not compared.

    RA_JIT_BIN=../target/release/ra-jit python3 doctests.py [--few] [name-version ...]

Each crate is unpacked by `vendor.sh` and built in a directory that is removed afterwards.
A difference is WRONG and fails the run. `--few` is the first three crates."""
import os
import re
import subprocess
import sys
import tempfile

from buildmodel import doctest_report, judge_doctests

HERE = os.path.dirname(os.path.abspath(__file__))
RA_JIT = os.path.abspath(os.environ.get("RA_JIT_BIN", os.path.join(HERE, "..", "target", "release", "ra-jit")))
CRATES = [
    "semver-1.0.28", "either-1.19.0", "smallvec-1.16.3",
    "serde_core-1.0.229", "regex-1.13.1", "itertools-0.15.0", "chrono-0.4.45", "anyhow-1.0.104",
    "bytes-1.12.1", "thiserror-2.0.21", "log-0.4.34", "bitflags-2.13.2", "indexmap-2.14.2",
    "once_cell-1.21.4", "hashbrown-0.17.1", "memchr-2.8.3", "url-2.5.8", "uuid-1.28.0",
    "time-0.3.55", "rand-0.10.3", "arrayvec-0.7.8", "base64-0.23.1",
    "byteorder-1.5.0", "num-traits-0.2.19", "strum-0.28.0", "nom-8.0.0", "textwrap-0.16.4",
    "unicode-segmentation-1.13.3", "httparse-1.10.1", "serde_json-1.0.154",
]


def tested(crate, command, target_dir):
    """How the crate's doctests ended, what they printed, and what was said beside."""
    path = os.path.dirname(RA_JIT) + os.pathsep + os.environ["PATH"]
    run = subprocess.run(["cargo", *command, "test", "--doc"], cwd=crate, capture_output=True, text=True,
                         timeout=3600, env={**os.environ, "PATH": path, "CARGO_TARGET_DIR": target_dir})
    return run.returncode, doctest_report(run.stdout), run.stderr


def row(spec):
    """The crate's line: its verdict and what there is to say of it."""
    subprocess.run(["sh", os.path.join(HERE, "vendor.sh"), spec], check=True)
    crate = os.path.join(HERE, "vendor", spec.rsplit("-", 1)[0])
    with tempfile.TemporaryDirectory(prefix="ra-jit-doctests-") as scratch:
        want = tested(crate, [], os.path.join(scratch, "cargo"))
        got = tested(crate, ["ra-jit"], os.path.join(scratch, "ra-jit"))
        # The second run has the programs of the first, and the store.
        again = tested(crate, ["ra-jit"], os.path.join(scratch, "ra-jit"))
    verdict = judged(want, got)
    if verdict[0] == "same" and want[:2] != again[:2]:
        return "WRONG", f"the second run differs: {judged(want, again)[1]}"
    return verdict


def judged(want, got):
    """What the two runs of a crate come to."""
    verdict, said = judge_doctests(want, got)
    if verdict == "same":
        tests = sum(l.startswith("test ") and not l.startswith("test result") for l in want[1])
        if not tests:
            return "WRONG", f"cargo ran no doctest: {want[2].strip()[-200:]}"
    return verdict, said


def main():
    args = [a for a in sys.argv[1:] if a != "--few"]
    counts = {"same": 0, "refused": 0, "WRONG": 0}
    for spec in args or (CRATES[:3] if "--few" in sys.argv else CRATES):
        verdict, said = row(spec)
        counts[verdict] += 1
        print(f"{spec:28} {os.environ.get('RA_JIT_BACKEND', 'cranelift'):9} {verdict:8} {said}", flush=True)
    print(f"{counts['same']} same, {counts['refused']} refused, {counts['WRONG']} WRONG")
    sys.exit(1 if counts["WRONG"] else 0)


if __name__ == "__main__":
    main()
