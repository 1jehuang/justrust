#!/usr/bin/env python3
"""bigbinary.py: edits spliced into a program larger than any one place of the splice is sized
for, as bevy is: no size of the program, of its code or of its frames is a reason to link in full
(#79).

The program is linked with an object made from assembly: 220 MB of code, and before it 800,000
functions of a frame each, over 16 MB of `__eh_frame` or `.eh_frame` that the unwinder reads
(written so that Apple's assembler can't encode them compactly). On Linux it is also linked with
an object of farfile.py's kind: 4.2 GB of `.debug_abbrev`, so that the file ends further from
its code than a debug build of bevy's does and the patch's DWARF says offsets four bytes can't.
So on AArch64 a branch from the program's functions does not reach where a splice adds the
patch, the tables of the unwinder are larger than a 24-bit offset says, and on Linux the file is
further from the code than four bytes of distance reach. Each step is compared with what `cargo
build` of the same source prints, and has to have been spliced: a function shorter than the long
jump, one that panics through its replacement, and one added.

usage: bigbinary.py   (RA_JIT_BIN names the ra-jit built). About 20 GB of disk on Linux.
"""
import os, sys, tempfile

from difftest import TARGET, host
from farfile import far_object
from includes import follow

AARCH64 = (TARGET or host()).startswith("aarch64")
APPLE = sys.platform == "darwin"
CODE = 220 << 20
FRAMES = 800_000
PIECES = 8
DEBUG = 4300 << 20


def assembly(path):
    """Writes the object's source to `path`: `ra_jit_big`, the functions of a frame each, and
    the code behind them in pieces. Apple's linker keeps a frame of each symbol's and puts
    islands for branches between them, so on macOS each function and piece is one, and none is
    dead-stripped."""
    prefix = "_" if APPLE else ""
    # An escape is what the compact encoding has none of, so each frame is in `__eh_frame`.
    frame = ".cfi_escape 0x2e, 0x00" if APPLE else ".cfi_def_cfa_offset 16"
    fill = 0 if AARCH64 else 0xcc

    def symbol(name):
        kept = f"\t.no_dead_strip {prefix}{name}\n" if APPLE else ""
        return f"{kept}{prefix}{name}:\n"

    with open(path, "w") as out:
        out.write(f"\t.text\n\t.globl {prefix}ra_jit_big\n\t.p2align 4\n")
        out.write(symbol("ra_jit_big") + "\t.cfi_startproc\n\tret\n\t.cfi_endproc\n")
        for nth in range(FRAMES):
            out.write(symbol(f"ra_jit_frame_{nth}") + f"\t.cfi_startproc\n\tnop\n\t{frame}\n\tret\n\t.cfi_endproc\n")
        for nth in range(PIECES):
            out.write(symbol(f"ra_jit_code_{nth}") + f"\t.fill {CODE // PIECES}, 1, {fill}\n")
        if APPLE:
            out.write("\t.subsections_via_symbols\n")


MAIN = "src/main.rs"
MAIN_TEXT = """extern "C" {
    fn ra_jit_big();
}

#[inline(never)]
fn nothing() {}

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
    near() + 1
}

fn extra() -> u64 {
    0
}

#[inline(never)]
fn risky(x: u64) -> u64 {
    if x > 2 {
        panic!("too big");
    }
    x
}

fn main() {
    if std::env::args().count() > 99 {
        unsafe { ra_jit_big() };
    }
    std::panic::set_hook(Box::new(|_| {}));
    nothing();
    let caught = std::panic::catch_unwind(|| risky(3)).is_err();
    println!("{} {} {} {}", long(3), through(), caught, extra());
}
"""
MANY = "".join(f"#[inline(never)]\nfn many_{n}(x: u64) -> u64 {{\n    x.wrapping_mul({n})\n}}\n" for n in range(3000))
MANY += "static MANY: [fn(u64) -> u64; 3000] = [" + ", ".join(f"many_{n}" for n in range(3000)) + "];\n"
MANY += "fn extra() -> u64 {\n    MANY.iter().map(|it| it(3)).fold(0, u64::wrapping_add)\n}\n"
STEPS = [
    ("nothing edited", [], False),
    ("a function's body", [(MAIN, "wrapping_mul(31)", "wrapping_mul(33)")], False, "spliced"),
    ("a function shorter than the long jump", [(MAIN, "fn nothing() {}", 'fn nothing() { println!("something"); }')], False, "spliced"),
    ("the caller of one in the executable", [(MAIN, "near() + 1", "near() + 2")], False, "spliced"),
    ("a function that panics through its replacement", [(MAIN, "if x > 2 {", "if x > 1 {")], False, "spliced"),
    ("that it no longer panics", [(MAIN, "if x > 1 {", "if x > 5 {")], False, "spliced"),
    # More frames than there is room for behind the program's.
    ("3,000 functions added", [(MAIN, "fn extra() -> u64 {\n    0\n}\n", MANY)], False, "spliced"),
    ("those taken back", [(MAIN, MANY, "fn extra() -> u64 {\n    0\n}\n")], False, "spliced"),
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
    if sys.platform not in ("linux", "darwin"):
        raise SystemExit("bigbinary.py: an ELF or Mach-O program, on Linux or macOS")
    scratch = tempfile.mkdtemp(prefix="ra-jit-bigbinary-")
    source = os.path.join(scratch, "big.s")
    assembly(source)
    args = [os.path.join(scratch, "big.o")]
    if not APPLE:
        args.append(os.path.join(scratch, "far.o"))
        far_object(args[-1], DEBUG, b".debug_abbrev")
    toml = """[package]
name = "sinc"
version = "0.1.0"
edition = "2021"
build = "build.rs"
"""
    # Room in the header for both tables of the unwinder, which `cargo ra-jit` gives a link.
    links = [*args, "-Wl,-headerpad,0x2400000"] if APPLE else args
    links = "".join(f'    println!("cargo:rustc-link-arg-bins={it}");\n' for it in links)
    build = f"""fn main() {{
    let built = std::process::Command::new("cc")
        .args(["-c", {source!r}, "-o", {args[0]!r}])
        .status()
        .expect("running cc");
    assert!(built.success(), "cc of {source} failed");
{links}}}
""".replace("'", '"')
    try:
        follow([("Cargo.toml", toml), ("build.rs", build), (MAIN, MAIN_TEXT)], STEPS)
    finally:
        for it in os.listdir(scratch):
            os.remove(os.path.join(scratch, it))
        os.rmdir(scratch)
