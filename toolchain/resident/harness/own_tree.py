#!/usr/bin/env python3
"""own_tree.py: `cargo blaze tree`, which blaze-tree makes with no cargo, against `cargo tree`,
over workspaces. cargo is the oracle; blaze never runs it outside this harness.

usage: own_tree.py [--blaze BIN] [--args "ARGS" ...] [--out FILE] DIR...

Each DIR is a workspace with its Cargo.lock, whose packages are fetched already (`cargo fetch`).
In each, every form of `--args` (by default the forms in FORMS) is run as `cargo tree ARGS` and
as `BIN cargo tree ARGS` (BIN: `ra-jit`, default `target/release/ra-jit`), both with
`--color never` and their output piped; standard output and the exit status are compared.
A case is equal, different, blaze-failed or cargo-failed; the first differing line of each
that is not equal is printed, `--out` writes them all as JSON, and the exit status is 1 when
any case is not equal.
"""
import argparse, json, os, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

FORMS = [
    "",
    "--workspace",
    "-e normal",
    "-e no-dev",
    "-e features",
    "-e all",
    "--target all",
    "--prefix depth",
    "--prefix none",
    "--no-dedupe --depth 3",
    "--depth 1",
    "--depth workspace --workspace",
    "-d",
    "-i syn --workspace",
    "--charset ascii",
    "-f {p}|{l}|{r}|{f}|{lib}",
    "--all-features",
    "--no-default-features",
    "-e no-proc-macro",
    "--workspace --prune serde",
    "-p serde",
    "-e build",
    "--target x86_64-pc-windows-msvc",
    "--target aarch64-apple-darwin -e normal",
    "-i serde_derive -e features",
    "--workspace -e features -i libc",
]


def run(argv, cwd):
    done = subprocess.run(argv, cwd=cwd, capture_output=True, text=True)
    return done.returncode, done.stdout, done.stderr


def first_difference(a, b):
    a_lines, b_lines = a.splitlines(), b.splitlines()
    for at, (x, y) in enumerate(zip(a_lines, b_lines)):
        if x != y:
            return f"line {at + 1}: cargo `{x}`, blaze `{y}`"
    if len(a_lines) != len(b_lines):
        return f"cargo prints {len(a_lines)} lines, blaze {len(b_lines)}"
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dirs", nargs="+")
    ap.add_argument("--blaze", default=os.path.join(ROOT, "target", "release", "ra-jit"))
    ap.add_argument("--args", action="append")
    ap.add_argument("--out")
    opts = ap.parse_args()
    forms = opts.args or FORMS
    cases = []
    for d in opts.dirs:
        for form in forms:
            args = ["tree", "--color", "never", *form.split()]
            cargo = run(["cargo", *args], d)
            blaze = run([opts.blaze, "cargo", *args], d)
            name = f"{os.path.basename(os.path.abspath(d))}: tree {form}".rstrip()
            if cargo[0] != 0 and blaze[0] == 0:
                verdict, why = "cargo-failed", cargo[2].strip().splitlines()[:3]
            elif cargo[0] == 0 and blaze[0] != 0:
                verdict, why = "blaze-failed", blaze[2].strip().splitlines()[:3]
            elif cargo[0] != blaze[0]:
                verdict, why = "different", [f"cargo exits {cargo[0]}, blaze {blaze[0]}"]
            elif cargo[0] != 0:
                same = cargo[2].strip() == blaze[2].strip()
                verdict = "equal" if same else "different"
                why = [] if same else [first_difference(cargo[2], blaze[2]) or ""]
            else:
                diff = first_difference(cargo[1], blaze[1])
                verdict, why = ("equal", []) if diff is None else ("different", [diff])
            cases.append({"case": name, "verdict": verdict, "why": why,
                          "lines": len(cargo[1].splitlines())})
            print(f"{verdict:13} {name}" + (f"  ({'; '.join(why)})" if why else ""), flush=True)
    equal = sum(1 for c in cases if c["verdict"] == "equal")
    print(f"\n{equal}/{len(cases)} equal")
    if opts.out:
        with open(opts.out, "w") as f:
            json.dump(cases, f, indent=1)
    return 0 if equal == len(cases) else 1


if __name__ == "__main__":
    sys.exit(main())
