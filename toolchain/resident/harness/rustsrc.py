#!/usr/bin/env python3
"""rustsrc.py: a toolchain without the standard library's source is turned down, saying so.

ra-jit compiles `core`, `alloc` and `std` from the `rust-src` component. Without it every call
into them is unresolved, and each function making one would be refused for a reason that says
nothing of the toolchain. The project is loaded with `RUST_SRC_PATH` naming a library with an
empty `core`, as one without the component's source looks to rust-analyzer: ra-jit has to fail
before building anything and say how the source is installed.
"""
import os, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from difftest import RA_JIT

TOML = """[package]
name = "nosrc"
version = "0.1.0"
edition = "2021"
"""
SAID = ("has no source of the standard library", "`rustup component add rust-src` installs it")


def main():
    with tempfile.TemporaryDirectory(prefix="ra-jit-rustsrc-") as work:
        project, library = os.path.join(work, "nosrc"), os.path.join(work, "library")
        os.makedirs(os.path.join(project, "src"))
        os.makedirs(os.path.join(library, "core"))
        open(os.path.join(project, "Cargo.toml"), "w").write(TOML)
        open(os.path.join(project, "src", "main.rs"), "w").write('fn main() {\n    println!("{}", 7);\n}\n')
        env = {**os.environ, "RUST_SRC_PATH": library}
        run = subprocess.run([RA_JIT, project, "--crate", "nosrc", "--out", os.path.join(work, "nosrc.o")],
                             capture_output=True, text=True, env=env, timeout=300)
        missing = [it for it in SAID if it not in run.stderr]
        if run.returncode == 0 or missing:
            print(f"ra-jit exited {run.returncode} without saying {missing}:\n{run.stderr[-2000:]}")
            sys.exit(1)
        if os.path.exists(os.path.join(work, "nosrc.o")):
            print("ra-jit wrote an object for a toolchain it turned down")
            sys.exit(1)
        print("turned down: " + next(line for line in run.stderr.splitlines() if SAID[0] in line).strip())


if __name__ == "__main__":
    main()
