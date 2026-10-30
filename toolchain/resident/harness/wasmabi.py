#!/usr/bin/env python3
"""wasmabi.py: the wasm signatures ra-jit gives `extern "C"` functions are rustc's.

What calls a wasm module from outside (wasm-bindgen's glue, a host) goes by the types of its
functions, so the C ABI of wasm is the module's interface. Builds `harness/wasmabi` with rustc
and with ra-jit for each wasm target and compares the type of every function the two objects
name, those they define and those they import.

usage: wasmabi.py [target ...]   (default: wasm32-unknown-unknown wasm32-wasip1)
"""
import os, re, subprocess, sys, tempfile
from difftest import RA_JIT

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(HERE, "wasmabi")
VALTYPES = {0x7f: "i32", 0x7e: "i64", 0x7d: "f32", 0x7c: "f64", 0x7b: "v128", 0x70: "funcref", 0x6f: "externref"}
# The symbol flags of the linking section.
UNDEFINED, EXPLICIT_NAME = 0x10, 0x40


def leb(data, at):
    value = shift = 0
    while True:
        byte = data[at]
        at += 1
        value |= (byte & 0x7f) << shift
        shift += 7
        if byte < 0x80:
            return value, at


def sysroot_rlibs(target):
    sysroot = subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
    lib = os.path.join(sysroot, "lib", "rustlib", target, "lib")
    return [os.path.join(lib, n) for n in sorted(os.listdir(lib)) if n.endswith(".rlib")]


def name(data, at):
    length, at = leb(data, at)
    return data[at:at + length].decode(), at + length


def limits(data, at):
    flags, at = leb(data, at)
    _, at = leb(data, at)
    if flags & 1:
        _, at = leb(data, at)
    return at


def signatures(path):
    """The type of each function an object names, by name, as `(i32 i32) -> (i32)`."""
    data = open(path, "rb").read()
    assert data[:4] == b"\0asm", path
    types, imported, defined, symbols, at = [], [], [], {}, 8
    while at < len(data):
        kind = data[at]
        size, at = leb(data, at + 1)
        end = at + size
        if kind == 1:
            count, p = leb(data, at)
            for _ in range(count):
                assert data[p] == 0x60
                params, p = leb(data, p + 1)
                ps, p = [VALTYPES[b] for b in data[p:p + params]], p + params
                results, p = leb(data, p)
                rs, p = [VALTYPES[b] for b in data[p:p + results]], p + results
                types.append(f"({' '.join(ps)}) -> ({' '.join(rs)})")
        elif kind == 2:
            count, p = leb(data, at)
            for _ in range(count):
                _, p = name(data, p)
                field, p = name(data, p)
                desc, p = data[p], p + 1
                if desc == 0:
                    index, p = leb(data, p)
                    imported.append((field, index))
                elif desc == 1:
                    p = limits(data, p + 1)
                elif desc == 2:
                    p = limits(data, p)
                elif desc == 3:
                    p += 2
                else:
                    _, p = leb(data, p + 1)
        elif kind == 3:
            count, p = leb(data, at)
            for _ in range(count):
                index, p = leb(data, p)
                defined.append(index)
        elif kind == 0:
            section, p = name(data, at)
            if section == "linking":
                _, p = leb(data, p)
                while p < end:
                    sub, p = data[p], p + 1
                    length, p = leb(data, p)
                    if sub == 8:
                        symbols = function_symbols(data, p)
                    p += length
        at = end
    of = lambda index: types[imported[index][1]] if index < len(imported) else types[defined[index - len(imported)]]
    return {sym or imported[index][0]: of(index) for index, sym in symbols}


def function_symbols(data, at):
    """The function symbols of a linking section's symbol table: (function index, name or None)."""
    count, at = leb(data, at)
    functions = []
    for _ in range(count):
        kind, at = data[at], at + 1
        flags, at = leb(data, at)
        if kind in (0, 2, 4, 5):
            index, at = leb(data, at)
            named = None
            if not flags & UNDEFINED or flags & EXPLICIT_NAME:
                named, at = name(data, at)
            if kind == 0:
                functions.append((index, named))
        elif kind == 1:
            _, at = name(data, at)
            if not flags & UNDEFINED:
                for _ in range(3):
                    _, at = leb(data, at)
        else:
            _, at = leb(data, at)
    return functions


def check(target, work):
    theirs, ours = os.path.join(work, f"{target}.rustc.o"), os.path.join(work, f"{target}.ra-jit.o")
    subprocess.run(["rustc", "--edition", "2024", "--crate-type", "lib", "--crate-name", "wasmabi", "-O",
                    "--emit", f"obj={theirs}", "--target", target, os.path.join(FIXTURE, "src/lib.rs")],
                   check=True)
    symbols = [x for rlib in sysroot_rlibs(target) for x in ("--symbols", rlib)]
    run = subprocess.run([RA_JIT, FIXTURE, "--crate", "wasmabi", "--out", ours, "--target", target, *symbols],
                         capture_output=True, text=True)
    if run.returncode != 0:
        print(f"{target}: ra-jit failed\n{run.stderr[-3000:]}")
        return False
    # The fixture's functions, not what either compiler calls of its own, as `memcpy`.
    source = open(os.path.join(FIXTURE, "src/lib.rs")).read()
    names = set(re.findall(r"\bfn (\w+)\(", source))
    want = {n: t for n, t in signatures(theirs).items() if n in names}
    got = signatures(ours)
    wrong = [n for n in sorted(want) if got.get(n) != want[n]]
    for n in wrong:
        print(f"  {target} {n}: rustc {want[n]}, ra-jit {got.get(n)}")
    print(f"{target}: {len(want) - len(wrong)} of {len(want)} functions typed as rustc's")
    return not wrong


def main():
    os.environ.setdefault("RUSTUP_TOOLCHAIN", open(os.path.join(HERE, "toolchain")).read().strip())
    targets = sys.argv[1:] or ["wasm32-unknown-unknown", "wasm32-wasip1"]
    with tempfile.TemporaryDirectory(prefix="wasmabi-") as work:
        right = sum(check(target, work) for target in targets)
    print(f"{right} of {len(targets)} right")
    sys.exit(right != len(targets))


if __name__ == "__main__":
    main()
