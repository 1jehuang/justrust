#!/usr/bin/env python3
"""storebuilds.py: a store one build of ra-jit wrote serves another build of the same sources
(#105).

The store keyed what it kept by the bytes of the ra-jit executable that wrote it, so a rebuild
of the same commit, on another machine or after a `touch`, started from an empty store, and a
CI runner that builds ra-jit for each commit reused nothing. It keys by what the build was made
from now: a hash of the workspace's sources, its features, target and whether debug
assertions are on, which `crates/ra-jit/build.rs` takes.

A daemon of RA_JIT_BIN builds the stress workspace into an empty store and ends. A copy of the
executable with other bytes, which is another build of the same sources, starts a daemon on
that store: it has to find nearly everything the first one compiled, and the program has to
print what cargo's does.

    python3 storebuilds.py
"""
import os, re, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import storelib
from storelib import Checkout, Daemon, scratch


def counts(daemon):
    """(functions found in the store, functions compiled and added) by a daemon's first build."""
    line = re.search(r"store: (\d+) functions found, (\d+) compiled and added", daemon.log())
    return (int(line.group(1)), int(line.group(2))) if line else (0, 0)


def another_build(exe, at):
    """A copy of `exe` whose bytes are not its: what a rebuild of the same sources is to the store."""
    shutil.copy2(exe, at)
    # A Mach-O with bytes past its signature fails codesign's checks, so on macOS the signature
    # is what differs.
    if sys.platform == "darwin":
        subprocess.run(["codesign", "--force", "--sign", "-", "--identifier", "another-build", at],
                       check=True, capture_output=True)
    else:
        with open(at, "ab") as copy:
            copy.write(b"\0another build of the same sources\0")
    return at


def main():
    work, store = scratch("ra-jit-storebuilds-"), scratch("ra-jit-storebuilds-store-")
    daemons, passed = [], False
    try:
        checkout = Checkout(work)
        first = Daemon(checkout, store, name="first")
        daemons.append(first)
        checkout.check(first, "the first build's daemon")
        first.kill()
        _, added = counts(first)
        if added == 0:
            raise SystemExit(f"FAIL: the first daemon added nothing to the store: {first.log_path}")

        storelib.RA_JIT = another_build(storelib.RA_JIT, os.path.join(work, os.path.basename(storelib.RA_JIT)))
        second = Daemon(checkout, store, name="second")
        daemons.append(second)
        checkout.check(second, "another build's daemon")
        second.kill()
        found, compiled = counts(second)
        if found == 0 or compiled * 20 > found:
            raise SystemExit(f"FAIL: another build of the same sources found {found} functions in the store the "
                             f"first wrote ({added} added) and compiled {compiled}: {second.log_path}")
        print(f"PASS: the first build added {added} functions; another build of the same sources found {found} "
              f"and compiled {compiled}")
        passed = True
    finally:
        for daemon in daemons:
            if daemon.process.poll() is None:
                daemon.kill()
        if passed:
            shutil.rmtree(work, ignore_errors=True)
            shutil.rmtree(store, ignore_errors=True)


if __name__ == "__main__":
    main()
