#!/usr/bin/env python3
"""Random edits of the kind people and agents make, thrown at one warm daemon on a real workspace.

usage: agents.py WORKSPACE PACKAGE CRATE[,CRATE...] [--steps N] [--seed S] [--tag T] [--project DIR] [--lib PATH:NAME]
                 [--noise FILE,FILE...] [-- RUN_ARG...]

The workspace is copied. Each step changes the copy, asks the daemon for a build with
`ra-jit sync`, builds the same source with cargo in a directory of its own and runs both programs:
they have to print the same. A step is one of
  - a unit added: a new module of the program (functions, a struct with derives, a trait with
    impls, generics, a macro, statics, closures, an `async fn`, collections, an enum, operators
    with `Drop` and associated types), with a line in `main` that prints what it computes;
  - a `user` unit, which uses a module the others of its kind share, and a change of that
    module: its struct's fields, its trait's methods, a signature, an enum's variants, a macro;
  - a unit changed: its next version, which differs in bodies, signatures, fields, variants,
    impls or instantiations;
  - a unit removed, file and all;
  - noise in the files named by --noise: a blank line at the top, a comment, a `let` in a body,
    a private function nothing calls;
  - with --lib, a public function added to that library crate (NAME is how `main` names the
    crate) or changed, called from `main`;
  - source that does not compile (the daemon has to say so and stay up), mended by the next step;
  - several of the above in one build;
  - back to the first source, and forward again.
Writes AGENTS-<package>.md.
"""
import os, random, re, shlex, shutil, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
os.environ.setdefault("RUSTUP_TOOLCHAIN", open(os.path.join(HERE, "toolchain")).read().strip())
import linkline
from difftest import Fixture, RA_JIT, sysroot_rlibs, RUNNER, profile_dir

MAIN = re.compile(r"fn main\(\)[^{;]*\{\n")
ITEM = re.compile(r"(use|mod|fn|pub|#\[|static|const|struct|enum|extern|type|macro_rules|impl|trait|async|unsafe)\b")
PLAIN_FN = re.compile(r"^ *(?:pub(?:\([a-z]+\))? )?fn \w+\([^)]*\)[^{;]*\{\n", re.M)
TYPES = ["7u8", "String::from(\"s\")", "(3u32, 'c')", "vec![1i64, 2]", "Some(\"t\")", "[1.5f64, 2.5]", "-9i128", "true"]


def plain(k, v):
    extra = "".join(f", p{i}: u32" for i in range(v % 3))
    args = "".join(f", {i + 2}" for i in range(v % 3))
    body = " + ".join([f"x * {k + 2}", str(v)] + [f"p{i}" for i in range(v % 3)])
    ret, wrap = ("u64", " as u64") if v % 4 == 3 else ("u32", "")
    return (f"pub fn value(x: u32{extra}) -> {ret} {{\n    helper({body}){wrap}\n}}\n\nfn helper(x: u32) -> u32 {{\n    x.wrapping_mul({v + 1}) ^ {k}\n}}\n",
            f"value(3{args})")


def struct(k, v):
    fields = [("id", "u32", str(k)), ("name", "String", f'"u{k}".to_string()'), ("weight", "f32", "1.5"), ("tags", "Vec<u8>", "vec![1, 2]"), ("next", "Option<Box<Record>>", "None")][: 2 + v % 4]
    derives = ["Debug", "Clone", "PartialEq"] + (["Default"] if v % 2 else []) + (["PartialOrd"] if v % 3 == 1 else [])
    decl = "".join(f"    pub {n}: {t},\n" for n, t, _ in fields)
    init = ", ".join(f"{n}: {e}" for n, _, e in fields)
    show = 'format!("{:?} {}", a, a == b)' if v % 3 != 1 else 'format!("{:?} {:?}", a, a.partial_cmp(&b))'
    return (f"#[derive({', '.join(derives)})]\npub struct Record {{\n{decl}}}\n\npub fn show() -> String {{\n    let a = Record {{ {init} }};\n    let mut b = a.clone();\n    b.id += {v};\n    {show}\n}}\n",
            "show()")


def trait(k, v):
    extra = "\n    fn name(&self) -> String {\n        format!(\"shape{}\", self.area())\n    }\n" if v % 2 else ""
    over = "\n    fn twice(&self) -> u32 {\n        self.area() * 3\n    }\n" if v % 3 == 1 else ""
    third = f"\npub struct Tri(pub u32);\n\nimpl Shape for Tri {{\n    const SIDES: u32 = 3;\n\n    fn area(&self) -> u32 {{\n        self.0 * {v}\n    }}\n}}\n" if v >= 2 else ""
    items = "Box::new(Sq(2)), Box::new(Rect(2, 5))" + (", Box::new(Tri(4))" if v >= 2 else "")
    name = ' + s.name().as_str()' if v % 2 else ""
    return (f"pub trait Shape {{\n    const SIDES: u32;\n\n    fn area(&self) -> u32;\n\n    fn twice(&self) -> u32 {{\n        self.area() * 2 + {v}\n    }}\n{extra}}}\n\npub struct Sq(pub u32);\npub struct Rect(pub u32, pub u32);\n\nimpl Shape for Sq {{\n    const SIDES: u32 = 4;\n\n    fn area(&self) -> u32 {{\n        self.0 * self.0\n    }}\n{over}}}\n\nimpl Shape for Rect {{\n    const SIDES: u32 = {4 + v};\n\n    fn area(&self) -> u32 {{\n        self.0 * self.1\n    }}\n}}\n{third}\nfn sides<T: Shape>(_: &T) -> u32 {{\n    T::SIDES\n}}\n\npub fn all() -> String {{\n    let shapes: Vec<Box<dyn ShapeDyn>> = vec![{items}];\n    let mut out = format!(\"{{}}\", sides(&Rect(1, 1)));\n    for s in &shapes {{\n        out += &format!(\" {{}}\", s.twice_dyn());\n    }}\n    out{''}\n}}\n\npub trait ShapeDyn {{\n    fn twice_dyn(&self) -> String;\n}}\n\nimpl<T: Shape> ShapeDyn for T {{\n    fn twice_dyn(&self) -> String {{\n        let s = self;\n        s.twice().to_string(){name}\n    }}\n}}\n",
            "all()")


def generic(k, v):
    # Not `+ &string`: a crate of the workspace may add to `String` too, and then `&String` fits no impl.
    uses = " + ".join(f"pick(&[{TYPES[(k + i) % len(TYPES)]}], {i})" + ".as_str()" for i in range(1 + v % 5))
    bound = "Clone + std::fmt::Debug" if v % 2 == 0 else "Clone + std::fmt::Debug + PartialEq"
    n = f"<const N: usize>(items: &[T; N]" if v % 3 == 2 else "(items: &[T]"
    gen = "<T: " + bound + (", const N: usize>" if v % 3 == 2 else ">")
    sig = f"fn pick{gen}(items: &[T{'; N' if v % 3 == 2 else ''}], n: usize) -> String"
    return (f"{sig} {{\n    let copy: Vec<T> = items.iter().cloned().cycle().take(n + 1).collect();\n    format!(\"{{:?}}/{{}}\", copy, std::mem::size_of::<T>())\n}}\n\npub fn picks() -> String {{\n    String::new() + {uses}\n}}\n",
            "picks()")


def macro(k, v):
    names = ", ".join(f"f{i}" for i in range(1 + v % 4))
    calls = " + ".join(f"f{i}()" for i in range(1 + v % 4))
    return (f"macro_rules! make {{\n    ($($name:ident),*) => {{\n        $(fn $name() -> usize {{\n            stringify!($name).len() + {v} + line!() as usize * 0\n        }})*\n    }};\n}}\n\nmacro_rules! sum {{\n    ($($x:expr),* $(,)?) => {{ 0usize $(+ $x)* }};\n}}\n\nmake!({names});\n\npub fn made() -> usize {{\n    sum!({calls}, {k}) + vec![1u8; {v + 1}].len() + matches!(Some({v}), Some(n) if n > 1) as usize\n}}\n",
            "made()")


def static(k, v):
    lazy = "\nstatic WORDS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();\n\nfn word(i: usize) -> &'static str {\n    &WORDS.get_or_init(|| \"zero one two three\".split(' ').map(String::from).collect())[i % 4]\n}\n" if v % 2 else "\nfn word(i: usize) -> &'static str {\n    [\"a\", \"b\"][i % 2]\n}\n"
    tls = "\nthread_local! {\n    static DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(%d) };\n}\n\nfn depth() -> u32 {\n    DEPTH.with(|d| {\n        d.set(d.get() + 1);\n        d.get()\n    })\n}\n" % v if v % 3 else "\nfn depth() -> u32 {\n    0\n}\n"
    return (f"use std::sync::atomic::{{AtomicU32, Ordering}};\n\nstatic COUNT: AtomicU32 = AtomicU32::new({v});\nconst TABLE: [u16; {2 + v % 3}] = [{', '.join(str(k + i) for i in range(2 + v % 3))}];\nstatic NAME: &str = \"unit{k}v{v}\";\n{lazy}{tls}\npub fn state() -> String {{\n    COUNT.fetch_add(TABLE.len() as u32, Ordering::Relaxed);\n    format!(\"{{}} {{}} {{}} {{}} {{}}\", COUNT.load(Ordering::Relaxed), NAME, word({v}), depth() + depth(), TABLE.iter().sum::<u16>())\n}}\n",
            "state()")


def closure(k, v):
    if v % 2:
        make = f"fn adder(by: u32) -> Box<dyn Fn(u32) -> u32> {{\n    Box::new(move |x| x + by + {v})\n}}\n"
    else:
        make = f"fn adder(by: u32) -> impl Fn(u32) -> u32 {{\n    move |x| x + by * {v + 1}\n}}\n"
    return (f"{make}\nfn apply<F: FnMut(u32) -> u32>(mut f: F, n: u32) -> u32 {{\n    (0..n).map(|i| f(i)).sum()\n}}\n\npub fn closures() -> u32 {{\n    let mut seen = Vec::new();\n    let total = apply(|x| {{\n        seen.push(x);\n        x * {k + 1}\n    }}, {3 + v});\n    let f: fn(u32) -> u32 = |x| x ^ {v};\n    adder(2)(total) + apply(adder(1), 3) + f(seen.len() as u32) + [1u32, 2, 3].iter().map(|x| x * {v}).filter(|x| x % 2 == 0).count() as u32\n}}\n",
            "closures()")


def asynch(k, v):
    awaits = "".join(f"    let x = step(x + {i}).await;\n" for i in range(1 + v % 4))
    return ("use std::future::Future;\nuse std::pin::pin;\nuse std::sync::Arc;\nuse std::task::{Context, Poll, Wake, Waker};\n\nstruct Noop;\n\nimpl Wake for Noop {\n    fn wake(self: Arc<Self>) {}\n}\n\nfn block_on<F: Future>(f: F) -> F::Output {\n    let waker = Waker::from(Arc::new(Noop));\n    let mut cx = Context::from_waker(&waker);\n    let mut f = pin!(f);\n    loop {\n        if let Poll::Ready(out) = f.as_mut().poll(&mut cx) {\n            return out;\n        }\n    }\n}\n\n"
            f"async fn step(x: u32) -> u32 {{\n    x * 2 + {v}\n}}\n\nasync fn chain(x: u32) -> u32 {{\n{awaits}    x\n}}\n\npub fn ran() -> u32 {{\n    block_on(chain({k})) + block_on(async {{ step({v}).await + 1 }})\n}}\n",
            "ran()")


def collections(k, v):
    return (f"use std::collections::{{BTreeMap, HashMap, VecDeque}};\n\npub fn tally() -> String {{\n    let text = \"the quick brown fox jumps over the lazy dog the end\";\n    let mut counts: HashMap<&str, usize> = HashMap::new();\n    for w in text.split(' ') {{\n        *counts.entry(w).or_default() += {1 + v};\n    }}\n    let sorted: BTreeMap<_, _> = counts.into_iter().filter(|(w, _)| w.len() > {v % 4}).collect();\n    let mut queue: VecDeque<u32> = (0..{k + 3}).collect();\n    queue.rotate_left({1 + v % 2});\n    let mut top: Vec<_> = sorted.iter().map(|(w, n)| (*n, *w)).collect();\n    top.sort_by(|a, b| b.cmp(a));\n    format!(\"{{:?}} {{:?}} {{}}\", &top[..2], queue.front(), sorted.len())\n}}\n",
            "tally()")


def enum(k, v):
    names = ["Start", "Stop", "Move", "Wait", "Jump", "Turn"][: 2 + v % 5]
    if v % 2:
        names.reverse()
    variants = "".join(f"    {n}{'(u32)' if i % 2 else ''},\n" for i, n in enumerate(names))
    arms = "".join(f"            Op::{n}{'(n)' if i % 2 else ''} => write!(f, \"{n.lower()}{{}}\", {'n' if i % 2 else i}),\n" for i, n in enumerate(names))
    build = ", ".join(f"Op::{n}{f'({i + v})' if i % 2 else ''}" for i, n in enumerate(names))
    return (f"use std::fmt;\n\n#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]\npub enum Op {{\n{variants}}}\n\nimpl fmt::Display for Op {{\n    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {{\n        match self {{\n{arms}        }}\n    }}\n}}\n\npub fn ops() -> String {{\n    let mut all = vec![{build}];\n    all.sort();\n    all.dedup();\n    let size = std::mem::size_of::<Option<Op>>();\n    all.iter().map(|o| o.to_string()).collect::<Vec<_>>().join(\",\") + &size.to_string()\n}}\n",
            "ops()")


def ops(k, v):
    add_int = "\nimpl Add<i32> for V2 {\n    type Output = V2;\n\n    fn add(self, o: i32) -> V2 {\n        V2(self.0 + o, self.1 - o)\n    }\n}\n" if v % 2 else ""
    use_int = "    let sum = sum + 1;\n" if v % 2 else ""
    third = '        let _c = Guard("c");\n' if v % 3 == 1 else ""
    return ("use std::cell::RefCell;\nuse std::ops::{Add, Index, Neg};\n\nthread_local! {\n    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };\n}\n\n#[derive(Clone, Copy, Debug, PartialEq)]\npub struct V2(pub i32, pub i32);\n\n"
            f"impl Add for V2 {{\n    type Output = V2;\n\n    fn add(self, o: V2) -> V2 {{\n        V2(self.0 + o.0 + {v}, self.1 + o.1)\n    }}\n}}\n{add_int}\nimpl Neg for V2 {{\n    type Output = V2;\n\n    fn neg(self) -> V2 {{\n        V2(-self.0, -self.1)\n    }}\n}}\n\npub struct Grid {{\n    cells: Vec<V2>,\n}}\n\nimpl Index<usize> for Grid {{\n    type Output = V2;\n\n    fn index(&self, i: usize) -> &V2 {{\n        &self.cells[i % self.cells.len()]\n    }}\n}}\n\nstruct Guard(&'static str);\n\nimpl Drop for Guard {{\n    fn drop(&mut self) {{\n        LOG.with(|l| l.borrow_mut().push(format!(\"drop {{}}\", self.0)));\n    }}\n}}\n\nstruct Count {{\n    at: u32,\n    to: u32,\n}}\n\nimpl Iterator for Count {{\n    type Item = u32;\n\n    fn next(&mut self) -> Option<u32> {{\n        if self.at < self.to {{\n            self.at += {1 + v % 3};\n            Some(self.at)\n        }} else {{\n            None\n        }}\n    }}\n}}\n\ntrait Named {{\n    fn name(&self) -> String;\n}}\n\ntrait Loud: Named {{\n    fn shout(&self) -> String {{\n        self.name().to_uppercase() + \"{'!' * (v % 3 + 1)}\"\n    }}\n}}\n\nimpl Named for V2 {{\n    fn name(&self) -> String {{\n        format!(\"v{{}}_{{}}\", self.0, self.1)\n    }}\n}}\n\nimpl Loud for V2 {{}}\n\ntrait Container {{\n    type Item;\n\n    fn first(&self) -> Option<Self::Item>;\n}}\n\nimpl Container for Grid {{\n    type Item = V2;\n\n    fn first(&self) -> Option<V2> {{\n        self.cells.{'first' if v % 2 == 0 else 'last'}().copied()\n    }}\n}}\n\nfn first_of<C: Container>(c: &C) -> Option<C::Item> {{\n    c.first()\n}}\n\npub fn ops() -> String {{\n    let g = Grid {{ cells: vec![V2(1, 2), V2(3, {k})] }};\n    let sum = g[0] + g[1] + -g[{v}];\n{use_int}    {{\n        let _a = Guard(\"a\");\n        let _b = Guard(\"b\");\n{third}    }}\n    let log = LOG.with(|l| l.borrow().join(\",\"));\n    let total: u32 = Count {{ at: 0, to: {5 + v} }}.filter(|x| x % 2 == 1).sum();\n    let loud: &dyn Loud = &sum;\n    format!(\"{{:?}} {{}} {{}} {{}} {{:?}}\", sum, log, total, loud.shout(), first_of(&g))\n}}\n",
            "ops()")


SHARED = 0


def shared_text(s):
    """A module the `user` units all use: what it declares changes shape with its version."""
    fields = "    pub scale: u32,\n" + ("    pub offset: u32,\n" if s % 3 >= 1 else "") + ("    pub label: &'static str,\n" if s % 3 == 2 else "")
    init = f"scale: {2 + s}" + (f", offset: {s}" if s % 3 >= 1 else "") + (f', label: "L{s}"' if s % 3 == 2 else "")
    describe = f"\n    fn describe(&self) -> String {{\n        String::from(\"d{s}\")\n    }}\n" if s % 2 else ""
    third = ", c: u32" if s % 4 >= 2 else ""
    ret, cast = ("u64", " as u64") if s % 4 == 3 else ("u32", "")
    mid = "    Mid(u8),\n" if s % 2 else ""
    mid_arm = "            Mode::Mid(n) => *n as u32,\n" if s % 2 else ""
    return (f"pub struct Config {{\n{fields}}}\n\nimpl Config {{\n    pub fn new() -> Config {{\n        Config {{ {init} }}\n    }}\n}}\n\npub trait Scaled {{\n    fn scaled(&self, c: &Config) -> u64;\n{describe}}}\n\npub fn combine(a: u32, b: u32{third}) -> {ret} {{\n    (a * {s + 1} + b{' + c' if third else ''}){cast}\n}}\n\nmacro_rules! shared_sum {{\n    ($($x:expr),*) => {{ 0u64 $(+ ($x as u64) * {s + 1})* }};\n}}\npub(crate) use shared_sum;\n\n#[allow(dead_code)]\npub enum Mode {{\n    Fast,\n    Slow,\n{mid}}}\n\nimpl Mode {{\n    pub fn cost(&self) -> u32 {{\n        match self {{\n            Mode::Fast => 1,\n            Mode::Slow => {s + 5},\n{mid_arm}        }}\n    }}\n}}\n\npub static BASE: u32 = {s * 7};\n\npub const fn konst() -> usize {{\n    {s % 5 + 1}\n}}\n")


def user(k, v):
    s = SHARED
    offset = " + c.offset as u64" if s % 3 >= 1 else ""
    label = " + c.label.len() as u64" if s % 3 == 2 else ""
    third = ", 3" if s % 4 >= 2 else ""
    describe = "Item(1).describe()" if s % 2 else '"-"'
    return (f"use super::ra_jit_ushared::{{self as shared, Config, Mode, Scaled}};\n\nstruct Item(u32);\n\nimpl Scaled for Item {{\n    fn scaled(&self, c: &Config) -> u64 {{\n        (self.0 * c.scale) as u64 + {v}{offset}{label}\n    }}\n}}\n\npub fn used() -> String {{\n    let c = Config::new();\n    let arr = [0u8; shared::konst()];\n    let comb = shared::combine({k}, {v}{third}) as u64;\n    let modes = [Mode::Fast, Mode::Slow];\n    format!(\"{{}} {{}} {{}} {{}} {{}} {{}}\", Item({k}).scaled(&c), comb, shared::shared_sum!(1, {v}), modes.iter().map(|m| m.cost()).sum::<u32>(), shared::BASE + arr.len() as u32, {describe})\n}}\n",
            "used()")


KINDS = {"plain": plain, "struct": struct, "trait": trait, "generic": generic, "macro": macro, "static": static,
         "closure": closure, "async": asynch, "collections": collections, "enum": enum,
         "ops": ops, "user": user, "user2": user}


class Source:
    """The copy of the workspace and what the steps have put into it."""

    def __init__(self, ws, main, lib, noise, rng):
        self.ws, self.main, self.lib, self.noise, self.rng = ws, main, lib, noise, rng
        self.units, self.libs, self.next, self.broken = {}, {}, 0, None
        self.shared = 0
        self.first = {p: open(p).read() for p in [main, *noise] + ([lib[0]] if lib else [])}
        self.text = dict(self.first)

    def unit_path(self, k):
        return os.path.join(os.path.dirname(self.main), f"ra_jit_u{k}.rs")

    def files(self):
        """Every file the steps own or change, as it should be now."""
        global SHARED
        SHARED = self.shared
        out = dict(self.text)
        mods, probes = "", ""
        if any(kind.startswith("user") for kind, _ in self.units.values()):
            out[os.path.join(os.path.dirname(self.main), "ra_jit_ushared.rs")] = shared_text(self.shared).replace("pub ", "pub(crate) ")
            mods += "mod ra_jit_ushared;\n"
        for k, (kind, v) in sorted(self.units.items()):
            text, call = KINDS[kind](k, v)
            # A program can deny `unreachable_pub`.
            out[self.unit_path(k)] = text.replace("pub ", "pub(crate) ")
            mods += f"mod ra_jit_u{k};\n"
            probes += f'    eprintln!("unit {k} {kind}: {{}}", ra_jit_u{k}::{call});\n'
        for k, v in sorted(self.libs.items()):
            args = ", ".join(str(i + 1) for i in range(1 + v % 3))
            params = ", ".join(f"p{i}: u64" for i in range(1 + v % 3))
            total = " + ".join(f"p{i}" for i in range(1 + v % 3))
            out[self.lib[0]] += f"\n/// Added.\npub fn ra_jit_lib_{k}({params}) -> u64 {{\n    ({total}) * {k + 2} + {v}\n}}\n"
            probes += f'    eprintln!("lib {k}: {{}}", {self.lib[1]}::ra_jit_lib_{k}({args}));\n'
        main = out[self.main]
        main = MAIN.sub(lambda m: m[0] + probes, main, 1)
        # After the inner attributes and doc comment of the crate, which have to come first.
        lines = main.split("\n")
        at = next(i for i, l in enumerate(lines) if ITEM.match(l))
        out[self.main] = "\n".join(lines[:at] + mods.rstrip("\n").split("\n") * bool(mods) + lines[at:])
        if self.broken:
            path, how = self.broken
            out[path] = how(out[path])
        return out

    def step(self):
        """Changes what the source should be, and says what was done and whether it compiles."""
        if self.broken:
            self.broken = None
            return "mended", True
        choices = ["add"] * 4 + ["noise"] * 3 + ["burst", "break"]
        if self.units:
            choices += ["bump"] * 6 + ["remove"]
        if self.lib:
            choices += ["lib"] * 2
        if any(kind.startswith("user") for kind, _ in self.units.values()):
            choices += ["shared"] * 4
        what = self.rng.choice(choices)
        if what == "burst":
            return "burst: " + "; ".join(self.one(self.rng.choice([c for c in choices if c not in ("burst", "break")])) for _ in range(self.rng.randint(2, 5))), True
        if what == "break":
            path = self.rng.choice(list(self.files()))
            how = self.rng.choice([
                ("syntax error", lambda t: t.replace("{", "{ (", 1)),
                ("type error", lambda t: t + "\nfn ra_jit_wrong() -> u32 {\n    \"text\"\n}\n"),
                ("unresolved name", lambda t: t + "\nfn ra_jit_wrong() -> u32 {\n    ra_jit_missing(1)\n}\n"),
                ("missing match arm", lambda t: t + "\n#[allow(dead_code)]\nfn ra_jit_wrong(x: Option<u8>) -> u8 {\n    match x {\n        Some(n) => n,\n    }\n}\n"),
                ("wrong argument count", lambda t: t + "\n#[allow(dead_code)]\nfn ra_jit_wrong() -> u32 {\n    ra_jit_two(1)\n}\n\n#[allow(dead_code)]\nfn ra_jit_two(a: u32, b: u32) -> u32 {\n    a + b\n}\n"),
                ("missing trait item", lambda t: t + "\n#[allow(dead_code)]\ntrait RaJitT {\n    fn f(&self);\n}\n\nimpl RaJitT for u8 {}\n"),
                ("missing field", lambda t: t + "\n#[allow(dead_code)]\nstruct RaJitS {\n    a: u8,\n    b: u8,\n}\n\n#[allow(dead_code)]\nfn ra_jit_wrong() -> RaJitS {\n    RaJitS { a: 1 }\n}\n"),
            ])
            self.broken = (path, how[1])
            return f"BROKEN: {how[0]} in {os.path.relpath(path, self.ws)}", False
        return self.one(what), True

    def one(self, what):
        rng = self.rng
        if what == "add" or what in ("bump", "remove") and not self.units:
            kind = rng.choice(list(KINDS))
            self.units[self.next] = (kind, 0)
            self.next += 1
            return f"unit {self.next - 1} added ({kind})"
        if what == "bump":
            k = rng.choice(list(self.units))
            kind, v = self.units[k]
            self.units[k] = (kind, v + 1)
            return f"unit {k} ({kind}) to version {v + 1}"
        if what == "remove":
            k = rng.choice(list(self.units))
            kind, _ = self.units.pop(k)
            return f"unit {k} ({kind}) removed"
        if what == "shared":
            self.shared += 1
            return f"shared module to version {self.shared}"
        if what == "lib":
            if self.libs and rng.random() < 0.6:
                k = rng.choice(list(self.libs))
                self.libs[k] += 1
                return f"library function {k} to version {self.libs[k]}"
            self.libs[self.next] = 0
            self.next += 1
            return f"library function {self.next - 1} added"
        path = rng.choice([self.main, *self.noise])
        text = self.text[path]
        n = self.next
        self.next += 1
        spots = [m.end() for m in PLAIN_FN.finditer(text)]
        how = rng.choice(["blank", "comment", "let", "fn"] if spots else ["comment", "fn"])
        if how == "blank":
            # Under whatever has to stay first in the file.
            at = spots[0] if text.startswith(("//!", "#![", "/*")) else 0
            at = text.rfind("\n", 0, text.rfind("\n", 0, at)) + 1 if at else 0
            self.text[path] = text[:at] + "\n" + text[at:]
        elif how == "comment":
            self.text[path] = text + f"// note {n}\n"
        elif how == "let":
            at = rng.choice(spots)
            self.text[path] = text[:at] + f"let _ra_jit_{n} = {n}u32;\n" + text[at:]
        else:
            self.text[path] = text + f"\n#[allow(dead_code)]\nfn ra_jit_unused_{n}(x: u32) -> u32 {{\n    x + {n}\n}}\n"
        return f"{how} in {os.path.relpath(path, self.ws)}"

    def write(self, files=None):
        files = files or self.files()
        folder = os.path.dirname(self.main)
        for name in os.listdir(folder):
            path = os.path.join(folder, name)
            if name.startswith("ra_jit_u") and path not in files:
                os.remove(path)
        for path, text in files.items():
            if not os.path.exists(path) or open(path).read() != text:
                open(path, "w").write(text)


def main():
    argv = sys.argv[1:]
    run_args = argv[argv.index("--") + 1:] if "--" in argv else []
    argv = argv[: argv.index("--")] if "--" in argv else argv
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    workspace, package, crates = argv[0], argv[1], argv[2].split(",")
    steps, seed = int(opt("--steps", "60")), int(opt("--seed", "1"))
    ws = tempfile.mkdtemp(prefix="ra-jit-agents-")
    shutil.copytree(workspace, ws, dirs_exist_ok=True, symlinks=True, ignore=shutil.ignore_patterns("target", "target-*", ".git"))
    work = tempfile.mkdtemp(prefix="ra-jit-agents-out-")
    obj, patch, binary = (os.path.join(work, n) for n in ("ra-jit.o", "ra-jit.patch.o", "bin-ra-jit"))
    socket, log_path, reference = os.path.join(work, "ra-jit.sock"), os.path.join(work, "daemon.log"), os.path.join(work, "reference")
    out = open(log_path, "a")
    fx = Fixture(ws, crates, ["-p", package])
    import json
    meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ws))
    bin_name = next(t["name"] for p in meta["packages"] if p["name"] == package for t in p["targets"] if "bin" in t["kind"])
    fx.target_args += ["--bin", bin_name]
    lib = opt("--lib")
    lib = (os.path.join(ws, lib.split(":")[0]), lib.split(":", 1)[1]) if lib else None
    noise = [os.path.join(ws, f) for f in (opt("--noise") or "").split(",") if f]
    source = Source(ws, fx.touch, lib, noise, random.Random(seed))

    def start():
        fx.rustc_build()
        symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
        link = list(fx.link_args)
        link = linkline.ahead(link, [patch, obj], RA_JIT)
        link[link.index("-o") + 1] = binary
        for path in (socket, binary):
            if os.path.exists(path):
                os.remove(path)
        seen = len(open(log_path).read())
        began = time.time()
        daemon = subprocess.Popen(
            [RA_JIT, os.path.join(ws, opt("--project", "")), *[x for c in crates for x in ("--crate", c)], "--out", obj, "--socket", socket,
             "--patch", patch, "--link-cmd", shlex.join(link), *symbols], stderr=out, stdout=out)
        while "watching" not in open(log_path).read()[seen:]:
            if daemon.poll() is not None:
                raise SystemExit(f"the daemon does not start, see {log_path}")
            time.sleep(0.05)
        return daemon, time.time() - began

    def run(path):
        try:
            p = subprocess.run([*RUNNER, path, *run_args], capture_output=True, text=True, timeout=30, stdin=subprocess.DEVNULL)
            # A usage line names the program by its file name, which is not the same for the two.
            return f"{p.stdout}\n--\n{p.stderr}\n[exit {p.returncode}]".replace(path, "BIN").replace("bin-ra-jit", bin_name)
        except Exception as error:
            return f"[{type(error).__name__}]"

    def rss(daemon):
        return int(re.search(r"VmRSS:\s+(\d+)", open(f"/proc/{daemon.pid}/status").read())[1]) // 1024

    daemon, warmup = start()
    print(f"warm in {warmup:.1f}s, rss {rss(daemon)} MB, log {log_path}, source {ws}", flush=True)
    rows, restarts, last = [], 0, None
    for index in range(steps):
        if index and index % 25 == 0 and not source.broken:
            name, compiles = "back to the first source", True
            last = source.files()
            source.write({**{p: t for p, t in source.first.items()}})
            pending = last
        elif last is not None:
            name, compiles = "forward again", True
            source.write(last)
            last = None
        else:
            name, compiles = source.step()
            source.write()
        asked = time.time()
        try:
            got = subprocess.run([RA_JIT, "sync", socket], capture_output=True, text=True, timeout=300)
            reply = got.stdout.strip() or got.stderr.strip().removeprefix("Error: daemon: ")
        except subprocess.TimeoutExpired:
            reply = "no reply in 300 s"
        took = (time.time() - asked) * 1000
        died = daemon.poll() is not None
        cargo = subprocess.run(["cargo", "build", "-q", "-p", package, "--bin", bin_name], cwd=ws, capture_output=True, text=True,
                               env={**os.environ, "CARGO_TARGET_DIR": reference})
        if (cargo.returncode == 0) != compiles:
            raise SystemExit(f"step {index} {name!r}: cargo {'failed' if cargo.returncode else 'passed'}:\n{cargo.stderr[-3000:]}")
        kind = "patch" if reply.startswith("ok patch") else "full" if reply.startswith("ok object") else "unchanged" if reply.startswith("ok unchanged") else "error"
        if died:
            verdict = "DAEMON DIED"
        elif not compiles:
            verdict = "said so" if kind == "error" else "ANSWERED OK: " + reply[:60]
        elif kind == "error":
            verdict = "REFUSED: " + reply[:300].replace("|", "/").replace("\n", " ")
        else:
            ours, theirs = run(binary), run(os.path.join(profile_dir(reference), bin_name))
            if ours == theirs:
                verdict = "same"
            else:
                a, b = ours.splitlines(), theirs.splitlines()
                diff = [f"`{x[:80]}` for `{y[:80]}`" for x, y in zip(a, b) if x != y][:3]
                verdict = "WRONG: " + ("; ".join(diff) or f"{len(a)} lines for {len(b)}")
        rows.append((index, name, kind, took, verdict))
        print(f"{index:3} {kind:9} {took:7.0f} ms  {verdict[:200]}  | {name[:150]}", flush=True)
        if died or verdict.startswith(("REFUSED", "WRONG", "ANSWERED")):
            if not died:
                daemon.kill()
                daemon.wait()
            open(log_path, "a").write(f"\n=== restart after step {index}\n")
            if compiles:
                daemon, _ = start()
            else:
                # The next step mends the source; the daemon starts from that.
                name2, _ = source.step()
                source.write()
                daemon, _ = start()
            restarts += 1
    memory = rss(daemon) if daemon.poll() is None else 0
    daemon.kill()
    good = sum(1 for r in rows if r[4] in ("same", "said so"))
    times = sorted(r[3] for r in rows if r[2] == "patch")
    summary = (f"{good} of {len(rows)} steps right, {restarts} restarts, {sum(r[2] == 'patch' for r in rows)} patches "
               f"(median {times[len(times) // 2] if times else 0:.0f} ms with the link), {sum(r[2] == 'full' for r in rows)} full objects, rss {memory} MB at the end")
    tag = opt("--tag")
    with open(os.path.join(HERE, f"AGENTS-{package}{'-' + tag if tag else ''}.md"), "w") as md:
        md.write(f"# Agent-style edits: {package}, seed {seed}\n\n{summary}. Warmup {warmup:.1f} s.\n\n| # | Edit | Reply | ms | Result |\n|---|---|---|---:|---|\n")
        for index, name, kind, took, verdict in rows:
            md.write(f"| {index} | {name} | {kind} | {took:.0f} | {verdict} |\n")
    print(f"{summary}; log {log_path}; source {ws}")


if __name__ == "__main__":
    main()
