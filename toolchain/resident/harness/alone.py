#!/usr/bin/env python3
"""alone.py: a program whose workspace crates rustc compiles nothing of.

usage: alone.py <workspace> <package> [crate ...]

cargo builds the workspace with ra-jit as `RUSTC_WORKSPACE_WRAPPER`, which leaves rustc the
registry crates and what runs during the build. ra-jit compiles the crates named (the package's
own by default) and the result is compared with a plain cargo build of the same source.
"""
import os, shutil, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from difftest import Fixture, RA_JIT


def host():
    return subprocess.check_output(["rustc", "-vV"], text=True).split("host: ")[1].split()[0]


class Alone(Fixture):
    """A fixture whose link line is of a build that has stand-ins for the workspace crates."""

    def rustc_build(self):
        self.alone = True
        super().rustc_build()


def main():
    source, package, *crates = sys.argv[1:]
    ws = tempfile.mkdtemp(prefix="ra-jit-alone-")
    shutil.copytree(source, ws, dirs_exist_ok=True, ignore=shutil.ignore_patterns("target*"))
    built = subprocess.run(["cargo", "build", "-p", package], cwd=ws, capture_output=True, text=True)
    assert built.returncode == 0, built.stderr[-2000:]
    fx = Alone(ws, [",".join(crates or [package.replace("-", "_")])], ["-p", package])
    fx.reference = os.path.join(ws, "target/debug", package)
    expected = fx.behaviour(fx.reference)
    fx.rustc_build()
    fx.ra_jit_build()
    rows = [r for rows in fx.reports.values() for r in rows]
    left = [r for r in rows if r[0] == "skipped"]
    print(f"{sum(r[0] == 'compiled' for r in rows)} functions compiled, {len(left)} not")
    for row in left[:20]:
        print("   ", *row[1:3])
    same = fx.matches(expected)
    print("same as cargo's" if same else "DIFFERENT from cargo's", fx.work)
    sys.exit(0 if same and not left else 1)


if __name__ == "__main__":
    main()
