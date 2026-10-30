#!/usr/bin/env python3
"""farfile.py: edits spliced into an ELF program whose file is gigabytes longer than what it
loads, as a debug build of a large program is: bevy's examples are 2 GB of what a debugger
reads behind 200 MB loaded (#79).

The program is linked with an object of one section that is not loaded, 2.3 GB of zeros, so
its file ends further from its code than four bytes of distance reach. A splice added behind
the file as far from it in memory as the program's frames are could not call into the program,
nor the program jump to it, and each step was linked in full ("the program is too large").
Each step is compared with what `cargo build` of the same source prints, and has to have been
spliced. Linux.
"""
import os, struct, sys, tempfile

from difftest import TARGET, host
from includes import follow

FAR = 2300 << 20
AARCH64 = (TARGET or host()).startswith("aarch64")


def far_object(path, size, name=b".ra_jit_far"):
    """A relocatable object with one section `name` of `size` zeros that is not loaded, written
    sparse."""
    names = b"\0" + name + b"\0.shstrtab\0"
    machine = 183 if AARCH64 else 62
    names_at, data_at = 64, 4096
    headers_at = data_at + size
    header = struct.pack("<16sHHIQQQIHHHHHH", b"\x7fELF\x02\x01\x01" + bytes(9), 1, machine, 1, 0, 0,
                         headers_at, 0, 64, 0, 0, 64, 3, 2)
    section = lambda name, kind, offset, length, align: struct.pack("<IIQQQQIIQQ", name, kind, 0, 0, offset, length, 0, 0, align, 0)
    with open(path, "wb") as f:
        f.write(header)
        f.seek(names_at)
        f.write(names)
        f.seek(headers_at)
        f.write(bytes(64) + section(1, 1, data_at, size, 1) + section(len(name) + 2, 3, names_at, len(names), 1))


MAIN = "src/main.rs"
MAIN_TEXT = """static TABLE: [u64; 4] = [3, 5, 8, 13];

#[inline(never)]
fn long(x: u64) -> u64 {
    let mut total = x;
    for i in 0..10 {
        total = total.wrapping_mul(31).wrapping_add(i);
    }
    total
}

#[inline(never)]
fn near() -> u64 {
    40
}

#[inline(never)]
fn through() -> u64 {
    near() + TABLE[1]
}

fn main() {
    println!("{} {} {}", long(3), through(), TABLE.iter().sum::<u64>());
}
"""
STEPS = [
    ("nothing edited", [], False),
    ("a function's body", [(MAIN, "wrapping_mul(31)", "wrapping_mul(33)")], False, "spliced"),
    ("again", [(MAIN, "wrapping_mul(33)", "wrapping_mul(35)")], False, "spliced"),
    ("the caller of one in the executable", [(MAIN, "near() + TABLE[1]", "near() + TABLE[2]")], False, "spliced"),
    ("a function added and called", [
        (MAIN, "fn main() {", "#[inline(never)]\nfn added(x: u64) -> u64 {\n    x * 7\n}\n\nfn main() {"),
        (MAIN, "long(3), through()", "long(added(3)), through()"),
    ], False, "spliced"),
    ("that taken back", [
        (MAIN, "#[inline(never)]\nfn added(x: u64) -> u64 {\n    x * 7\n}\n\n", ""),
        (MAIN, "long(added(3)), through()", "long(3), through()"),
    ], False, "spliced"),
]

if __name__ == "__main__":
    if sys.platform != "linux":
        raise SystemExit("farfile.py: an ELF program, on Linux")
    far = os.path.join(tempfile.mkdtemp(prefix="ra-jit-farfile-"), "far.o")
    far_object(far, FAR)
    toml = """[package]
name = "sinc"
version = "0.1.0"
edition = "2021"
build = "build.rs"
"""
    build = f'fn main() {{\n    println!("cargo:rustc-link-arg-bins={far}");\n}}\n'
    try:
        follow([("Cargo.toml", toml), ("build.rs", build), (MAIN, MAIN_TEXT)], STEPS)
    finally:
        os.remove(far)
