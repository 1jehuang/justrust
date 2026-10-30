#!/usr/bin/env python3
"""wholeprogram.py: programs ra-jit builds whole, with a `main` of its own, run as cargo's do.

usage: wholeprogram.py [CASE...] [--release] [--keep]

Each case is a program of the workspace `harness/wholeprogram`, copied to a directory of
its own. Cargo builds it as it is, and again with ra-jit as `RUSTC_WRAPPER` and
`RUSTC_WORKSPACE_WRAPPER` as `runbench.py` builds a program: rustc builds the registry's
crates and stand-ins of no code for the workspace's, and ra-jit compiles every crate of the
program into one object, with the C `main` of it, which is linked in their place. Both
programs run, and what they print on either stream and the status they exit with have to be
the same. ra-jit then builds the object once more, and it has to be the same bytes: a whole
program's layout follows the order its functions were asked for, not the order the threads
finished them (#200).

  default_alloc  the default allocator; output on both streams and a status of its own
  custom_alloc   a `#[global_allocator]` of the program that counts what it is asked for
  lib_alloc      the `#[global_allocator]` is in a library of the workspace
  cfg_alloc      a `#[global_allocator]` under a `#[cfg]` that is false, as ripgrep's
  trait_block    a method a trait provides calls a function declared in its body
  c_main         `#![no_main]` and a `main` of C's, as bun's, with a body of 16 384 statements

The backend is `RA_JIT_BACKEND`'s; `--release` builds both with the release profile. With
LLVM, a build of a whole program, whichever `main` it has, leaves no function to Cranelift
for being large or for its handlers, as a build that edits follow does.
"""
import argparse, os, shutil, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import apps

CASES = ["default_alloc", "custom_alloc", "lib_alloc", "cfg_alloc", "trait_block", "c_main"]
# What `backend::Backend::emit` leaves to Cranelift only in a build that is not of a whole program.
NOT_WHOLE = ("a body larger than LLVM is given in a build", "a handler LLVM lowers only for a tier")


def check(ws, case, release):
    spec = {"package": case, "cargo_args": ["--release"] if release else []}
    fx = apps.App(ws, spec, alone=True)
    fx.ra_jit_args = ["--release"] if release else []
    try:
        fx.rustc_build()
        fx.ra_jit_build()
    except SystemExit as failed:
        return f"  ra-jit's build failed: {str(failed)[-800:]}"
    log = open(os.path.join(fx.work, "all.stderr")).read()
    left = [line.strip() for line in log.splitlines() if line.strip().endswith(NOT_WHOLE)]
    if left:
        return "  not built as a whole program, LLVM left to Cranelift:\n" + "\n".join(f"    {it}" for it in left)
    program, _ = apps.reference(ws, {"package": case}, "release" if release else "debug")
    theirs, ours = fx.behaviour(program), fx.behaviour(fx.binary)
    lines = []
    for what, a, b in zip(("status", "stdout", "stderr"), theirs, ours):
        if a != b:
            lines.append(f"  {what}: cargo's {a!r}, ra-jit's {b!r}")
    if lines:
        return "\n".join(lines)
    # The same source has to give the same object every build, whichever thread compiled
    # which function first (#200).
    first = open(fx.objects[0], "rb").read()
    try:
        fx.ra_jit_build()
    except SystemExit as failed:
        return f"  ra-jit's second build failed: {str(failed)[-800:]}"
    if open(fx.objects[0], "rb").read() != first:
        return f"  built differently the second time: {fx.objects[0]} ({len(first)} bytes the first)"
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("cases", nargs="*", default=CASES)
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--keep", action="store_true", help="keep the copy of the workspace")
    args = parser.parse_args()
    work = tempfile.mkdtemp(prefix="ra-jit-wholeprogram-")
    ws = os.path.join(work, "ws")
    shutil.copytree(os.path.join(HERE, "wholeprogram"), ws, ignore=shutil.ignore_patterns("target*"))
    failed = 0
    for case in args.cases:
        wrong = check(ws, case, args.release)
        print(f"{case:15} {'WRONG' if wrong else 'same'}")
        if wrong:
            print(wrong)
            failed += 1
    if args.keep:
        print(f"kept {ws}")
    else:
        shutil.rmtree(work, ignore_errors=True)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
