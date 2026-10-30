#!/usr/bin/env python3
"""store.py: generated programs and changed asm! bodies against an empty and a warm store.

usage: store.py [--backends cranelift,llvm]

Issue #418: smoke-1/c16 differed only with RA_JIT_CACHE set. Two asm! wrappers can have
identical Cranelift IR but different assembly, so LLVM must key its code by the assembly too
(2a3a0a35). Compile one body, change only its assembly, then read each body back from the
store. The smoke seeds also run twice, first filling a fresh store and then reading it.
RA_JIT_BIN names the compiler under test. A failure keeps its work and logs.
"""
import argparse, os, shutil, subprocess, sys, tempfile

from fuzz import BACKENDS, Project

HERE = os.path.dirname(os.path.abspath(__file__))
ASM = """    #[cfg(target_arch = "x86_64")]
    #[inline(never)]
    fn transform(mut x: u64, mut y: u64) -> u64 {
        unsafe { std::arch::asm!(X86, x = inout(reg) x, y = inout(reg) y => _, options(pure, nomem, nostack)); }
        x
    }
    #[cfg(target_arch = "aarch64")]
    #[inline(never)]
    fn transform(mut x: u64, mut y: u64) -> u64 {
        unsafe { std::arch::asm!(ARM, x = inout(reg) x, y = inout(reg) y => _, options(pure, nomem, nostack)); }
        x
    }
    pub fn run() {
        for (x, y) in [(16u64, 55u64), (u64::MAX, 1), (0, 0), (0x8000_0000_0000_0000, 16)] {
            feed(transform(black_box(x), black_box(y)));
        }
    }
"""
ASSEMBLY = {
    "add": ('"add {x}, {y}"', '"add {x}, {x}, {y}"'),
    "negate": ('"neg {x}", "not {y}", "add {x}, {y}"',
               '"neg {x}, {x}", "mvn {y}, {y}", "add {x}, {x}, {y}"'),
}


def asm_changes(work, backend):
    """One IR with two different asm! bodies, each compiled and restored in a new process."""
    os.environ["RA_JIT_CACHE"] = os.path.join(work, "asm-store")
    project = Project(os.path.join(work, "asm"))
    for step, body in enumerate(("add", "negate", "add", "negate")):
        x86, arm = ASSEMBLY[body]
        lines = ASM.replace("X86", x86).replace("ARM", arm).splitlines()
        rejected = project.reference([("c16", lines)], "2021")
        if rejected is not None:
            raise SystemExit(rejected)
        findings, _ = project.ra_jit(backend)
        shutil.copyfile(os.path.join(project.fx.work, "fz.stderr"),
                        os.path.join(work, f"asm-{step}-{body}.stderr"))
        if findings:
            raise SystemExit(f"asm {step} {body} ({backend}):\n" +
                             "\n".join(f.detail for f in findings))
        print(f"MATCH asm {step} {body} ({backend})", flush=True)
    shutil.rmtree(project.fx.work)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--backends", default="cranelift,llvm")
    args = parser.parse_args()
    backends = args.backends.split(",")
    for backend in backends:
        if backend not in BACKENDS:
            parser.error(f"unknown backend: {backend}")
    work = tempfile.mkdtemp(prefix="ra-jit-fuzz-store-")
    print(f"work: {work}", flush=True)
    for backend in backends:
        own = os.path.join(work, backend)
        os.environ["RA_JIT_HOME"] = os.path.join(own, "home")
        asm_changes(own, backend)
        os.environ["RA_JIT_CACHE"] = os.path.join(own, "smoke-store")
        for phase in ("cold", "warm"):
            print(f"smoke {phase} ({backend})", flush=True)
            command = [sys.executable, os.path.join(HERE, "fuzz.py"), "smoke", "--backends", backend,
                       "--reduce", "0", "--work", os.path.join(own, "smoke"),
                       "--out", os.path.join(own, f"smoke-{phase}")]
            status = subprocess.run(command).returncode
            if status:
                raise SystemExit(status)
    shutil.rmtree(work)


if __name__ == "__main__":
    main()
