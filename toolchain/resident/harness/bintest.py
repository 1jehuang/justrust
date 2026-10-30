#!/usr/bin/env python3
"""Differential test of a program: the same command lines given to cargo's binary and ra-jit's.

usage: bintest.py <name> [--json out.json]

<name> is an entry of breakers/bins.json: the checkout (BINS_DIR/<name>, cloned from `repo` when
missing), the package whose binary is linked, the workspace crates ra-jit compiles, the command
lines and the patterns of output that differs from run to run (`scrub`). Each command line runs in the checkout; stdout, stderr and exit status must match.
"""
import argparse, json, os, re, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from difftest import Fixture, compare, discard, symptom

BINS_DIR = os.environ.get("BINS_DIR") or os.path.join(HERE, "vendor", "bins")


class Program(Fixture):
    """Both binaries are run under the program's name: usage text prints `argv[0]`."""

    def behaviour(self, binary):
        try:
            run = subprocess.run([self.argv0, *self.run_args], executable=binary, capture_output=True,
                                 stdin=subprocess.DEVNULL, timeout=120)
        except subprocess.TimeoutExpired:
            return "timeout", b"", b""
        return run.returncode, self.scrub(run.stdout), self.scrub(run.stderr)

    def scrub(self, output):
        """What a program measures (elapsed times) differs from run to run."""
        for pattern in self.scrubbed:
            output = re.sub(pattern.encode(), b"<scrubbed>", output)
        return output


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("name")
    ap.add_argument("--json")
    a = ap.parse_args()
    spec = json.load(open(os.path.join(HERE, "breakers", "bins.json")))[a.name]
    ws = os.path.join(BINS_DIR, a.name)
    if not os.path.isdir(ws):
        os.makedirs(BINS_DIR, exist_ok=True)
        subprocess.run(["git", "clone", "-q", "--depth", "1", spec["repo"], ws], check=True)
    if a.json:
        a.json = os.path.abspath(a.json)
    os.chdir(ws)
    runs = spec["runs"]
    fx = Program(ws, [",".join(spec["crates"])], ["-p", spec["package"], *spec.get("cargo_args", [])])
    fx.argv0, fx.scrubbed = spec["package"], spec.get("scrub", [])
    fx.run_args = runs[0]
    began = time.monotonic()
    result = compare(fx, a.name, False)
    result["seconds"] = round(time.monotonic() - began, 1)
    differing = [] if result["match"] else [" ".join(runs[0])]
    for run in runs[1:]:
        fx.run_args = run
        want, got = fx.behaviour(fx.reference), fx.behaviour(fx.binary)
        if want != got:
            differing.append(" ".join(run))
            status = f"rustc status {want[0]}, ra-jit status {got[0]}"
            tail = got[2].decode(errors="replace").strip().splitlines()[-2:]
            print(f"  DIFFERS `{' '.join(run)}`: {status}: {symptom(want, got)} {' | '.join(tail)[:300]}")
    result.update(match=not differing, differing_tests=differing, tests_run=len(runs))
    print(f"{'MATCH' if not differing else 'MISMATCH'}: {len(runs) - len(differing)} of {len(runs)} command lines the same")
    if a.json:
        merged = {"fixture": a.name, "match": result["match"], "tests_run": len(runs),
                  "crates": result["crates"], "binaries": [result]}
        json.dump(merged, open(a.json, "w"), indent=1)
    discard([result])
    sys.exit(0 if result["match"] else 1)


if __name__ == "__main__":
    main()
