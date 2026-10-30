#!/usr/bin/env python3
"""frozenwarm.py: frozen def maps of a crate whose items an `include!` reads (#40, #161).

A dependency under a registry's path, so that its `DefMap` is frozen, has its items in files
`include!` reads and the VFS does not load at first, as the icu `*_data` crates do: its
`lib.rs` includes `../data/mod.rs`, which declares a `macro_rules!`, exports another and
includes `items.rs.data`, whose items the macros write and which exports a macro of its own.
A program calls those items and both exported macros, and an item of a second dependency that
its build script writes into `OUT_DIR`. ra-jit builds it, with nothing of rustc's, on one store:

  - cold: the def maps are frozen and written;
  - warm, twice: they are served, and the program is built and prints what cargo's does
    (a def map served before the files it was read from were loaded broke on its
    `macro_rules!`, "Can't find ast id MacroRules");
  - warm with `RA_JIT_FROZEN=check` and with `=verify`: what was frozen is what resolving finds,
    and what reading it writes back;
  - another checkout of the same source at another path, whose build script writes below its
    own target directory, with the first's put away: it is served the def maps of as many
    crates as the first, read from what its own build script wrote (#105);
  - warm after `items.rs.data` changed at the same path: the frozen def map is not served, and
    the program prints what cargo's new one does.

    python3 frozenwarm.py
"""
import os, re, shutil, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from allrajit import linked, outcome, ra_jit, run
from storelib import scratch

LIB = 'include!("../data/mod.rs");\n'
MOD = """macro_rules! made_by {
    ($name:ident, $value:expr) => { pub fn $name() -> u32 { $value } };
}
#[macro_export]
macro_rules! shout {
    ($text:expr) => { format!("{}!", $text) };
}
include!("items.rs.data");
"""
ITEMS = """made_by!(made, 7);
made_by!(other, 35);
pub const TABLE: &[u32] = &[1, 2, 3];
#[macro_export]
macro_rules! from_data {
    () => { 5 };
}
"""
MAIN = """fn main() {
    let sum: u32 = incdata::TABLE.iter().sum();
    println!("{} {} {}", incdata::made() + incdata::other(), sum, incdata::shout!("frozen"));
    println!("{} {}", incdata::from_data!(), gendata::generated());
}
"""
# A dependency whose items its build script writes, below the target directory of whichever
# checkout builds it.
GEN_BUILD = """fn main() {
    let out = std::env::var("OUT_DIR").unwrap();
    std::fs::write(format!("{out}/gen.rs"), "pub fn generated() -> u32 { 11 }\\n").unwrap();
}
"""
GEN_LIB = 'include!(concat!(env!("OUT_DIR"), "/gen.rs"));\n'


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def frozen_line(ran):
    said = re.findall(r"^frozen, of .*$", ran.stderr, re.M)
    return said[0] if said else ""


def main():
    if not sys.platform.startswith("linux"):
        print("not run: the programs ra-jit links with nothing of rustc's are Linux's so far")
        return
    work = scratch("ra-jit-frozenwarm-")
    passed = False
    try:
        dep = os.path.join(work, "home/registry/src/index.crates.io-0000000000000000/incdata-0.1.0")
        write(os.path.join(dep, "Cargo.toml"), '[package]\nname = "incdata"\nversion = "0.1.0"\nedition = "2021"\n')
        write(os.path.join(dep, "src/lib.rs"), LIB)
        write(os.path.join(dep, "data/mod.rs"), MOD)
        write(os.path.join(dep, "data/items.rs.data"), ITEMS)
        gen = os.path.join(work, "home/registry/src/index.crates.io-0000000000000000/gendata-0.1.0")
        write(os.path.join(gen, "Cargo.toml"), '[package]\nname = "gendata"\nversion = "0.1.0"\nedition = "2021"\n')
        write(os.path.join(gen, "build.rs"), GEN_BUILD)
        write(os.path.join(gen, "src/lib.rs"), GEN_LIB)
        root = os.path.join(work, "app")
        write(os.path.join(root, "Cargo.toml"), '[package]\nname = "app"\nversion = "0.1.0"\nedition = "2021"\n'
              f'[dependencies]\nincdata = {{ path = "{dep}" }}\ngendata = {{ path = "{gen}" }}\n[workspace]\n')
        write(os.path.join(root, "src/main.rs"), MAIN)
        subprocess.run(["cargo", "generate-lockfile", "-q"], cwd=root, check=True)
        store = {"RA_JIT_CACHE": os.path.join(work, "store")}
        target = os.path.join(root, "target")

        def cargo_says():
            run(["cargo", "build", "-q", "--target-dir", target], root, "cargo build")
            return outcome(os.path.join(target, "debug/app"), [], root)

        lines = []

        def build(what, served, refused=0, at=root, **env):
            """`served`: whether def maps come from the store; `refused`: how many of the
            parts the store has do not fit what is loaded. `at`: the checkout. Returns how
            many crates' def maps were served."""
            obj, binary = os.path.join(work, "app.o"), os.path.join(work, "app.bin")
            ran, took = ra_jit(at, ["app"], obj, "--exe", binary, env={**store, **env})
            linked(ran, what)
            frozen = frozen_line(ran)
            if served != bool(re.search(r"def maps of [1-9]", frozen)):
                raise SystemExit(f"FAIL: {what}: the def maps should {'' if served else 'not '}"
                                 f"have been served: {frozen!r}\n{ran.stderr}")
            unreadable = re.search(r"(\d+) naming a file not loaded", frozen)
            if (int(unreadable[1]) if unreadable else 0) != refused:
                raise SystemExit(f"FAIL: {what}: {refused} frozen parts should not have fit: {frozen!r}")
            got = outcome(binary, [], at)
            if got != want:
                raise SystemExit(f"FAIL: {what}: the program differs from cargo's:\n"
                                 f"cargo:  {want}\nra-jit: {got}")
            lines.append(f"{what}: {took:.2f} s, {frozen}")
            return int(re.search(r"def maps of (\d+)", frozen)[1])

        want = cargo_says()
        build("cold", served=False)
        served = build("warm", served=True)
        build("warm again", served=True)
        build("warm, checked", served=True, RA_JIT_FROZEN="check")
        build("warm, verified", served=True, RA_JIT_FROZEN="verify")
        # Another checkout of the same source elsewhere, whose build script writes below its own
        # target directory: every frozen crate is the first checkout's (#105).
        # The first's target directory is put away meanwhile, so nothing it serves names a file
        # its build script wrote there.
        other = os.path.join(work, "elsewhere", "app")
        shutil.copytree(root, other, ignore=shutil.ignore_patterns("target"))
        os.rename(os.path.join(root, "target"), os.path.join(work, "first-target"))
        elsewhere = build("another checkout", served=True, at=other)
        os.rename(os.path.join(work, "first-target"), os.path.join(root, "target"))
        if elsewhere != served:
            raise SystemExit(f"FAIL: another checkout was served the def maps of {elsewhere} crates, the first of {served}")
        write(os.path.join(dep, "data/items.rs.data"), ITEMS.replace("made_by!(made, 7)", "made_by!(made, 8)"))
        want = cargo_says()
        build("warm, the included file changed", served=True, refused=1)
        print("\n".join(lines))
        print("PASS: frozen def maps of a crate whose items are included, served warm as resolved")
        passed = True
    finally:
        if passed:
            shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
