#!/usr/bin/env python3
"""growth.py: the code of one function grows in a straight line with its source.

usage: growth.py [ARMS [FIELDS]]

Builds a program whose `serialize` is the shape `#[derive(Serialize)]` gives an internally tagged
enum: a match of ARMS arms (default 160), each opening a value with a destructor through `?`, then
`?` on every field of the variant. It is built again with twice the arms, and ra-jit's `serialize`
in the second has to be less than GROWTH_RATIO (default 2.6) times the size of the first. A
straight line doubles; issue #5 was 4 KB per arm squared, 329 MB of code for 285 arms, out of
reach of an AArch64 branch.

Then the same of a `load` that fills a struct of FIELDS fields (default 200), each `lib.symbol(..)?`
while a value with a destructor is live, as x11-dl's `x11_link!` opens Xlib's 781 functions: every
`?` leaves through what the fields before it hold. Leaving was a drop chain per `?` over every
field before it, 3 blocks a field squared, and bevy's build with Xlib's took 364 GB. Every program
is also run against cargo's.

Both sizes are allocated by the single-pass register allocator. By default a function of more
than SINGLE_PASS instructions (hir-ty-codegen) gets it and a smaller one the backtracking one,
whose code is smaller: with the limit between the two sizes, the ratio measured the
change of allocator, not the growth (Windows' load of 200 fields, under the limit since #561,
took x2.63 the code at 400 that were over it; the 400 were the same 288 KB as before).
"""
import os, shutil, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
os.environ["RA_JIT_CACHE"] = "off"
os.environ["RA_JIT_REGALLOC"] = "single_pass"
from difftest import Fixture

CARGO_TOML = """[package]
name = "growth"
version = "0.1.0"
edition = "2021"

[workspace]
"""

HEAD = """use std::fmt::Write;

pub struct Error(pub String);

pub struct Out<'a>(pub &'a mut String);

pub struct State<'a> {
    out: &'a mut String,
    open: bool,
}

impl Drop for State<'_> {
    fn drop(&mut self) {
        if self.open {
            self.out.push('!');
        }
    }
}

pub trait Ser: Sized {
    type St: St;
    fn begin(self, name: &'static str, len: usize) -> Result<Self::St, Error>;
}

pub trait St {
    fn field<T: std::fmt::Display + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Error>;
    fn end(self) -> Result<(), Error>;
}

impl<'a> Ser for Out<'a> {
    type St = State<'a>;
    fn begin(self, name: &'static str, len: usize) -> Result<State<'a>, Error> {
        if len > 1000 {
            return Err(Error(name.to_owned()));
        }
        self.0.push('{');
        Ok(State { out: self.0, open: true })
    }
}

impl St for State<'_> {
    fn field<T: std::fmt::Display + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<(), Error> {
        if key.is_empty() {
            return Err(Error(String::new()));
        }
        write!(self.out, "{key}:{value},").map_err(|_| Error(String::new()))
    }

    fn end(mut self) -> Result<(), Error> {
        self.open = false;
        self.out.push('}');
        Ok(())
    }
}
"""


def program(arms):
    variants = "".join(f"    V{i} {{ id: String, n: u32 }},\n" if i % 2 else f"    V{i},\n" for i in range(arms))
    cases = "".join(
        f'            Action::V{i} {{ ref id, ref n }} => {{\n'
        f'                let mut state = serializer.begin("Action", 3)?;\n'
        f'                state.field("type", "a/v{i}")?;\n'
        f'                state.field("id", id)?;\n'
        f'                state.field("n", n)?;\n'
        f'                state.end()\n'
        f'            }}\n' if i % 2 else
        f'            Action::V{i} => {{\n'
        f'                let mut state = serializer.begin("Action", 1)?;\n'
        f'                state.field("type", "a/v{i}")?;\n'
        f'                state.end()\n'
        f'            }}\n'
        for i in range(arms))
    return (HEAD + f"\npub enum Action {{\n{variants}}}\n\nimpl Action {{\n"
            "    pub fn serialize<S: Ser>(&self, serializer: S) -> Result<(), Error> {\n"
            f"        match *self {{\n{cases}        }}\n    }}\n}}\n\n"
            "fn main() {\n    let mut out = String::new();\n"
            f'    for action in [Action::V0, Action::V1 {{ id: "x".to_owned(), n: 3 }}, Action::V{arms - 1} {{ id: "y".to_owned(), n: 4 }}] {{\n'
            "        if action.serialize(Out(&mut out)).is_err() {\n            out.push('?');\n        }\n    }\n"
            '    println!("{out}");\n}\n')


FIELDS_HEAD = """pub struct OpenError(pub String);

pub struct Lib(Vec<u8>);

impl Drop for Lib {
    fn drop(&mut self) {
        println!("closed {}", self.0.len());
    }
}

impl Lib {
    fn symbol(&self, name: &str) -> Result<usize, OpenError> {
        if name == "missing" { Err(OpenError(name.to_owned())) } else { Ok(name.len() + self.0.len()) }
    }
}
"""


def fields_program(fields):
    names = [f"f{i}" for i in range(fields)]
    names[-1] = "missing"
    decls = "".join(f"    pub f{i}: usize,\n" for i in range(fields))
    inits = "".join(f'        f{i}: lib.symbol("{name}")?,\n' for i, name in enumerate(names))
    return (FIELDS_HEAD + f"\npub struct Funcs {{\n{decls}}}\n\n"
            "pub fn load(missing: bool) -> Result<(Lib, Funcs), OpenError> {\n"
            "    let lib = Lib(vec![1; 3]);\n"
            "    if !missing {\n"
            "        return Err(OpenError(\"not asked\".to_owned()));\n"
            "    }\n"
            f"    let funcs = Funcs {{\n{inits}    }};\n"
            "    Ok((lib, funcs))\n}\n\n"
            "fn main() {\n"
            "    for missing in [false, true] {\n"
            "        match load(missing) {\n"
            '            Ok((_, funcs)) => println!("{}", funcs.f0),\n'
            '            Err(e) => println!("error {}", e.0),\n'
            "        }\n    }\n}\n")


def code_size(shapes, function):
    """The bytes of the last `function` the shapes log has code for."""
    names, sizes = {}, {}
    for line in open(shapes):
        tag, *rest = line.rstrip("\n").split("\t")
        if tag == "O":
            names[rest[0]] = rest[3]
        elif tag == "C":
            sizes[rest[0]] = int(rest[1])
    found = [size for symbol, size in sizes.items() if names.get(symbol) == function]
    if not found:
        raise SystemExit(f"no code for `{function}` in the shapes log")
    return max(found)


def build(source, function, what):
    ws = tempfile.mkdtemp(prefix="ra-jit-growth-")
    os.makedirs(os.path.join(ws, "src"))
    open(os.path.join(ws, "Cargo.toml"), "w").write(CARGO_TOML)
    open(os.path.join(ws, "src/main.rs"), "w").write(source)
    fx = Fixture(ws, ["growth"], ["-p", "growth"])
    # What is quadratic takes minutes and gigabytes: past this it has failed already.
    fx.ra_jit_timeout = 600
    fx.rustc_build()
    shapes = os.path.join(fx.work, "shapes.tsv")
    os.environ["RA_JIT_SHAPES"] = shapes
    try:
        fx.ra_jit_build()
    finally:
        del os.environ["RA_JIT_SHAPES"]
    expected, got = fx.behaviour(fx.reference), fx.behaviour(fx.binary)
    if expected != got:
        raise SystemExit(f"{what}: ra-jit's program differs from cargo's:\n  rustc: {expected}\n  ra-jit: {got}")
    size = code_size(shapes, function)
    shutil.rmtree(ws, ignore_errors=True)
    return size


def grows(function, unit, n, program, limit):
    small = build(program(n), function, f"{n} {unit}s")
    large = build(program(2 * n), function, f"{2 * n} {unit}s")
    ratio = large / small
    print(f"{function}: {small:,} bytes for {n} {unit}s, {large:,} for {2 * n}: x{ratio:.2f} "
          f"({small // n:,} and {large // (2 * n):,} bytes a {unit})")
    if ratio > limit:
        raise SystemExit(f"FAIL: twice the {unit}s is x{ratio:.2f} the code, more than x{limit}")


def main():
    arms = int(sys.argv[1]) if sys.argv[1:] else 160
    fields = int(sys.argv[2]) if sys.argv[2:] else 200
    limit = float(os.environ.get("GROWTH_RATIO", "2.6"))
    grows("serialize", "arm", arms, program, limit)
    grows("load", "field", fields, fields_program, limit)
    print("ok")


if __name__ == "__main__":
    main()
