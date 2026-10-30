#!/usr/bin/env python3
"""wasmsections.py: the custom sections of a wasm object ra-jit writes are rustc's.

A `#[link_section]` static is a custom section of the module on wasm, which wasm-bindgen and
the like read. Builds `harness/wasmsections` with rustc and with ra-jit for each wasm target
and compares the custom sections of the two objects, leaving out the linker's own and those of
tools.

usage: wasmsections.py [target ...]   (default: wasm32-unknown-unknown wasm32-wasip1)
"""
import os, subprocess, sys, tempfile
from difftest import RA_JIT

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(HERE, "wasmsections")
# The linker's, and what tools put in: not the program's.
TOOLS = ("linking", "name", "producers")


def leb(data, at):
    value = shift = 0
    while True:
        byte = data[at]
        at += 1
        value |= (byte & 0x7f) << shift
        shift += 7
        if byte < 0x80:
            return value, at


def custom_sections(path):
    """The custom sections of a wasm module or object, by name, those of one name joined."""
    data = open(path, "rb").read()
    assert data[:4] == b"\0asm", path
    sections, at = {}, 8
    while at < len(data):
        kind = data[at]
        size, at = leb(data, at + 1)
        end = at + size
        if kind == 0:
            length, start = leb(data, at)
            name = data[start:start + length].decode()
            if name not in TOOLS and not name.startswith(("reloc.", ".debug")):
                sections[name] = sections.get(name, b"") + data[start + length:end]
        at = end
    return sections


def sysroot_rlibs(target):
    sysroot = subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
    lib = os.path.join(sysroot, "lib", "rustlib", target, "lib")
    return [os.path.join(lib, n) for n in sorted(os.listdir(lib)) if n.endswith(".rlib")]


def check(target, work):
    theirs, ours = os.path.join(work, f"{target}.rustc.o"), os.path.join(work, f"{target}.ra-jit.o")
    subprocess.run(["rustc", "--edition", "2024", "--crate-type", "lib", "--crate-name", "wasmsections",
                    "--emit", f"obj={theirs}", "--target", target, os.path.join(FIXTURE, "src/lib.rs")],
                   check=True)
    symbols = [x for rlib in sysroot_rlibs(target) for x in ("--symbols", rlib)]
    run = subprocess.run([RA_JIT, FIXTURE, "--crate", "wasmsections", "--out", ours, "--target", target, *symbols],
                         capture_output=True, text=True)
    if run.returncode != 0:
        print(f"{target}: ra-jit failed\n{run.stderr[-3000:]}")
        return False
    want, got = custom_sections(theirs), custom_sections(ours)
    if want == got:
        print(f"{target}: {len(want)} custom sections, as rustc's")
        return True
    print(f"{target}: DIFFERENT")
    for name in sorted(set(want) | set(got)):
        if want.get(name) != got.get(name):
            print(f"  {name}: rustc {want.get(name)!r}, ra-jit {got.get(name)!r}")
    return False


def main():
    os.environ.setdefault("RUSTUP_TOOLCHAIN", open(os.path.join(HERE, "toolchain")).read().strip())
    targets = sys.argv[1:] or ["wasm32-unknown-unknown", "wasm32-wasip1"]
    with tempfile.TemporaryDirectory(prefix="wasmsections-") as work:
        right = sum(check(target, work) for target in targets)
    print(f"{right} of {len(targets)} right")
    sys.exit(right != len(targets))


if __name__ == "__main__":
    main()
