#!/usr/bin/env python3
"""asmedits.py: a daemon follows edits to `asm!` blocks and to the code around them.

The blocks are written for the architecture of the target. Each step is compared with what `cargo build` of the same source prints.
"""
from difftest import TARGET, host
from includes import follow

# The same blocks in the assembly of each architecture.
ASM = {
    "x86_64": {
        "scale2": '"lea {out}, [{x} + {x} * 2]"', "scale4": '"lea {out}, [{x} + {x} * 4]"',
        "add": '"add {0:e}, {1:e}", inout(reg) a, in(reg) b',
        "add_const": '"add {0:e}, {1:e}", "add {0:e}, {2}", inout(reg) a, in(reg) b, const 9',
        "add_named": '"add eax, {0:e}", "add eax, {1}", in(reg) b, const 9, inout("eax") a',
        "not": '"not {0:e}", inout(reg) a',
    },
    "aarch64": {
        "scale2": '"add {out}, {x}, {x}, lsl #1"', "scale4": '"add {out}, {x}, {x}, lsl #2"',
        "add": '"add {0:w}, {0:w}, {1:w}", inout(reg) a, in(reg) b',
        "add_const": '"add {0:w}, {0:w}, {1:w}", "add {0:w}, {0:w}, #{2}", inout(reg) a, in(reg) b, const 9',
        "add_named": '"add w0, w0, {0:w}", "add w0, w0, #{1}", in(reg) b, const 9, inout("x0") a',
        "not": '"mvn {0:w}, {0:w}", inout(reg) a',
    },
}[(TARGET or host()).split("-")[0]]

MAIN = "src/main.rs"
MAIN_TEXT = """use std::arch::asm;

fn scale(x: u64) -> u64 {
    let out: u64;
    unsafe { asm!(%(scale2)s, x = in(reg) x, out = lateout(reg) out) };
    out
}

fn sum(a: u32, b: u32) -> u32 {
    let mut a = a;
    unsafe { asm!(%(add)s) };
    a + 1
}

fn main() {
    println!("{} {}", scale(7), sum(3, 4));
}
""" % ASM
# (what the step is, [(file, old, new)], whether cargo rejects it)
STEPS = [
    ("nothing edited", [], False),
    ("an instruction", [(MAIN, ASM["scale2"], ASM["scale4"])], False),
    ("the code around a block", [(MAIN, "a + 1", "a + 2")], False),
    ("an operand added", [
(MAIN, ASM["add"], ASM["add_const"])
    ], False),
    ("a block made the same as another", [(MAIN, ASM["scale4"], ASM["scale2"])], False),
    ("a block that does not type check", [(MAIN, "in(reg) x,", "in(reg) missing,")], True),
    ("mended", [(MAIN, "in(reg) missing,", "in(reg) x,")], False),
    ("a named register", [
(MAIN, ASM["add_const"], ASM["add_named"])
    ], False),
    ("a block added", [
        (MAIN, "    a + 2", f'    unsafe {{ asm!({ASM["not"]}) }};\n    a + 2'),
    ], False),
    ("a block removed", [(MAIN, f'    unsafe {{ asm!({ASM["not"]}) }};\n', "")], False),
]

if __name__ == "__main__":
    follow([(MAIN, MAIN_TEXT)], STEPS)
