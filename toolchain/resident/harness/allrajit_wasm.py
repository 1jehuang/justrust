#!/usr/bin/env python3
"""allrajit_wasm.py: wasm programs with nothing rustc built in them (docs/no-rustc.md, step G).

allrajit.py's programs that a wasm32-wasip1 program can be (a hello, the rich one without its
threads and its caught panics, as wasm's panics abort, and the one built with
`panic = "abort"`) are built by ra-jit for wasm32-wasip1 with every crate its own, the
standard library included, and linked by ra-jit (`--exe`) with the toolchain's `rust-lld`
and the wasi-libc the toolchain ships (C, as glibc is on Linux). A wasm32-unknown-unknown
`cdylib` is linked too (`--shared`), and a `#![no_std]` guest of a host with its own panic
handler and simd128/relaxed-simd code, each export called with `wasmtime run --invoke`; both
are built again by `cargo blaze build`, with the `cargo-blaze` beside ra-jit. Nothing
ra-jit could not compile may be left in a module. What each prints and how it exits, under
`wasmtime run`, has to be what cargo's build of the same source does; the sizes of both are
printed.

The standard library of each target is compiled once into a store that starts empty, as a
unit, and taken from it by the programs after the first.

    python3 allrajit_wasm.py           # needs wasmtime, and rustup's wasm32-wasip1 and
                                       # wasm32-unknown-unknown targets
"""
import os, re, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from difftest import RA_JIT
from storelib import scratch

WASI = "wasm32-wasip1"
UNKNOWN = "wasm32-unknown-unknown"
WASMTIME = os.environ.get("RA_JIT_WASMTIME", "wasmtime")

HELLO = """fn main() {
    let v: Vec<u32> = (1..=10).collect();
    println!("hello {} {:?}", v.iter().sum::<u32>(), std::env::args().count());
}
"""
RICH = r"""use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::io::Write as _;

#[derive(Debug, Clone, PartialEq)]
enum Shape { Circle(f64), Rect { w: f64, h: f64 } }

trait Area { fn area(&self) -> f64; }
impl Area for Shape {
    fn area(&self) -> f64 {
        match self { Shape::Circle(r) => std::f64::consts::PI * r * r, Shape::Rect { w, h } => w * h }
    }
}

fn main() {
    let mut words = HashMap::new();
    for w in "the quick brown fox jumps over the lazy dog the end".split(' ') {
        *words.entry(w).or_insert(0usize) += 1;
    }
    let sorted: BTreeMap<_, _> = words.iter().collect();
    println!("words {:?}", sorted);
    let shapes: Vec<Box<dyn Area>> = vec![Box::new(Shape::Circle(1.5)), Box::new(Shape::Rect { w: 2.0, h: 3.25 })];
    let total: f64 = shapes.iter().map(|s| s.area()).sum();
    println!("area {total:.6} {:e} {}", total, 1.0f64 / 3.0);
    println!("shape {:?}", Shape::Rect { w: 1.0, h: 2.5 });
    let mut s = String::new();
    write!(s, "{:>8}|{:<6}|{:^7}|{:#x}|{:08.3}", "right", "left", "mid", 255, -3.14159).unwrap();
    println!("{s}");
    let text = std::fs::read_to_string("Cargo.toml").unwrap();
    println!("lines {}", text.lines().count());
    println!("parsed {:?} {:?}", "42".parse::<i64>(), "x".parse::<u8>().is_err());
    std::io::stdout().flush().unwrap();
    if std::env::args().count() > 1 { let none: Option<u8> = None; none.expect("asked to fail"); }
}
"""
ABORT = r"""fn main() {
    let caught = std::panic::catch_unwind(|| 1);
    println!("caught {:?}", caught.ok());
    if std::env::args().count() > 1 { panic!("aborting {}", 3); }
}
"""
# (program, its source, the runs to compare: the arguments of each, its panic strategy)
PROGRAMS = [("hello", HELLO, [["a", "b"]], "unwind"), ("rich", RICH, [[], ["fail"]], "unwind"),
            ("abort", ABORT, [[], ["fail"]], "abort")]
# A library of a page or a host's guest: no system, its functions called by name.
# Its own allocator over a static, until ra-jit compiles `llvm.wasm.memory.grow` (sub-lane
# `simd`): dlmalloc, the target's, grows the memory with it.
LIBRARY = r"""use std::alloc::{GlobalAlloc, Layout};
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

struct Bump { heap: Cell<[u8; 1 << 20]>, used: Cell<usize> }
unsafe impl Sync for Bump {}
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let start = (self.used.get() + layout.align() - 1) & !(layout.align() - 1);
        if start + layout.size() > 1 << 20 { return std::ptr::null_mut(); }
        self.used.set(start + layout.size());
        unsafe { self.heap.as_ptr().cast::<u8>().add(start) }
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}
#[global_allocator]
static HEAP: Bump = Bump { heap: Cell::new([0; 1 << 20]), used: Cell::new(0) };

thread_local! {
    static SEEN: RefCell<BTreeMap<u64, String>> = RefCell::new(BTreeMap::new());
}

#[unsafe(no_mangle)]
pub extern "C" fn allrajit_count(n: u64) -> u64 {
    let text: String = (0..n).map(|i| format!("{i},")).collect();
    SEEN.with(|seen| seen.borrow_mut().insert(n, text.clone()));
    let seen = SEEN.with(|seen| seen.borrow().len() as u64);
    seen * 1000 + text.len() as u64 * 10
}

#[unsafe(no_mangle)]
pub extern "C" fn allrajit_mix(a: u32, b: u32) -> u32 {
    a.rotate_left(b % 32) ^ b.wrapping_mul(0x9e37_79b9)
}
"""
CALLS = [("allrajit_count", ["1"]), ("allrajit_count", ["10"]), ("allrajit_mix", ["7", "35"])]
# A host's guest as indexable-inc/wasm-host's are: `#![no_std]` with a panic handler of its own,
# so that its standard library is core, compiler_builtins and alloc and not std, and SIMD
# (simd128, relaxed-simd) through `core::arch::wasm32`.
GUEST = r"""#![no_std]
extern crate alloc;
use alloc::vec::Vec;
use core::alloc::{GlobalAlloc, Layout};
use core::arch::wasm32::*;
use core::cell::UnsafeCell;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! { unreachable() }

struct Bump { heap: UnsafeCell<[u8; 1 << 16]>, used: UnsafeCell<usize> }
unsafe impl Sync for Bump {}
unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let used = unsafe { &mut *self.used.get() };
        let start = (*used + layout.align() - 1) & !(layout.align() - 1);
        if start + layout.size() > 1 << 16 { return core::ptr::null_mut(); }
        *used = start + layout.size();
        unsafe { self.heap.get().cast::<u8>().add(start) }
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}
#[global_allocator]
static HEAP: Bump = Bump { heap: UnsafeCell::new([0; 1 << 16]), used: UnsafeCell::new(0) };

static TABLE: [u32; 16] = [3, 1, 4, 1, 5, 9, 2, 6, 5, 3, 5, 8, 9, 7, 9, 3];

#[unsafe(no_mangle)]
pub extern "C" fn guest_dot(a: i32, b: i32) -> i32 {
    let x = i32x4(a, a.wrapping_add(1), a.wrapping_mul(3), a.wrapping_neg());
    let p = i32x4_mul(x, i32x4_splat(b));
    let s = i32x4_add(p, i32x4_shuffle::<3, 2, 1, 0>(p, p));
    i32x4_extract_lane::<0>(s) ^ i32x4_extract_lane::<1>(s).rotate_left(7)
}

#[unsafe(no_mangle)]
pub extern "C" fn guest_sum(k: u32) -> u32 {
    let mut acc = u32x4_splat(0);
    for i in 0..4 {
        let v = unsafe { v128_load(TABLE.as_ptr().add(i * 4).cast::<v128>()) };
        acc = i32x4_add(acc, i32x4_mul(v, u32x4_splat(k)));
    }
    let lanes: Vec<u32> = (0..4).map(|i| match i {
        0 => u32x4_extract_lane::<0>(acc), 1 => u32x4_extract_lane::<1>(acc),
        2 => u32x4_extract_lane::<2>(acc), _ => u32x4_extract_lane::<3>(acc),
    }).collect();
    lanes.iter().fold(0u32, |h, &it| h.wrapping_mul(31).wrapping_add(it))
}

#[unsafe(no_mangle)]
pub extern "C" fn guest_bytes(n: u32) -> u32 {
    let v = u8x16_splat(n as u8);
    let w = u8x16_add_sat(v, u8x16(0, 16, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240));
    let s = i8x16_swizzle(w, u8x16(15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0));
    let m = u8x16_bitmask(i8x16_gt(s, i8x16_splat(0))) as u32;
    m | (u8x16_all_true(s) as u32) << 16 | (u8x16_extract_lane::<0>(s) as u32) << 24
}

#[unsafe(no_mangle)]
pub extern "C" fn guest_floats(a: f32, b: f32) -> u32 {
    let x = f32x4(a, b, a * b, a - b);
    let y = f32x4_sqrt(f32x4_abs(x));
    let r = f32x4_relaxed_madd(x, y, f32x4_pmax(x, y));
    let t = i32x4_relaxed_trunc_f32x4(f32x4_mul(r, f32x4_splat(64.0)));
    (f32x4_extract_lane::<0>(r) + f32x4_extract_lane::<3>(r)).to_bits()
        ^ i32x4_extract_lane::<2>(t) as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn guest_wide(a: i64, b: i64) -> i64 {
    let v = i64x2_mul(i64x2(a, b), i64x2(b, a.wrapping_add(3)));
    let high = ((a as u128).wrapping_mul(b as u128) >> 64) as i64;
    i64x2_extract_lane::<0>(v) ^ i64x2_extract_lane::<1>(i64x2_shr(v, 5)) ^ high
}
"""
GUEST_CONFIG = '[target.wasm32-unknown-unknown]\nrustflags = ["-C", "target-feature=+simd128,+relaxed-simd"]\n'
GUEST_CALLS = [("guest_dot", ["7", "-3"]), ("guest_dot", ["2147483647", "2"]), ("guest_sum", ["5"]),
               ("guest_bytes", ["9"]), ("guest_bytes", ["200"]), ("guest_floats", ["1.5", "-2.25"]),
               ("guest_wide", ["-81985529216486895", "1311768467463790320"])]


def run(cmd, cwd, what, env=None):
    start = time.monotonic()
    # cargo's build is read from the package's own target directory.
    env = {**os.environ, "RUST_BACKTRACE": "0", **(env or {})}
    for name in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET", "RUSTFLAGS"):
        env.pop(name, None)
    ran = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, env=env)
    took = time.monotonic() - start
    if ran.returncode != 0:
        raise SystemExit(f"FAIL: {what} exited {ran.returncode}:\n{ran.stdout}{ran.stderr}")
    return ran, took


def outcome(module, args, cwd):
    """How the module runs under wasmtime: its status, what it printed, and what it said up to
    the trap of an abort, whose backtrace names functions as each compiler named them."""
    ran = subprocess.run([WASMTIME, "run", "--dir", ".", module, *args], cwd=cwd,
                         capture_output=True, text=True, env={**os.environ, "RUST_BACKTRACE": "0"})
    said = ran.stderr.split("Error: failed to run main module")[0]
    return ran.returncode, ran.stdout, said


def invoked(module, cwd, calls):
    """What each of the calls answers, each in a process of its own."""
    answers = []
    for name, args in calls:
        ran = subprocess.run([WASMTIME, "run", "--invoke", name, module, *args], cwd=cwd,
                             capture_output=True, text=True)
        answers.append((ran.returncode, ran.stdout.strip()))
    return answers


def package(work, name, manifest, path, source, config=None):
    root = os.path.join(work, name)
    os.makedirs(os.path.join(root, "src"))
    if config:
        os.makedirs(os.path.join(root, ".cargo"))
        with open(os.path.join(root, ".cargo", "config.toml"), "w") as f:
            f.write(config)
    with open(os.path.join(root, "Cargo.toml"), "w") as f:
        f.write(f'[package]\nname = "{name}"\nversion = "0.1.0"\nedition = "2021"\n{manifest}[workspace]\n')
    with open(os.path.join(root, "src", path), "w") as f:
        f.write(source)
    subprocess.run(["cargo", "generate-lockfile", "-q"], cwd=root, check=True)
    return root


def ra_jit(root, crate, out, exe, target, store, *extra):
    env = {"RA_JIT_CACHE": store, "RA_JIT_STD_FROM": "off"}
    return run([RA_JIT, root, "--crate", crate, "--out", out, "--exe", exe, "--target", target,
                *extra], root, f"ra-jit {crate} ({target})", env)


def cargo_blaze(root, what, store, runtime, target):
    """`cargo blaze build` as a user runs it, the `cargo-blaze` beside ra-jit first on the path
    and a daemon of its own, in a copy of the package without cargo's build. Returns the copy
    and how long the build took."""
    env = {"PATH": os.path.dirname(os.path.abspath(RA_JIT)) + os.pathsep + os.environ["PATH"],
           "RA_JIT_CACHE": store, "XDG_RUNTIME_DIR": runtime}
    copy = root + "-blaze"
    shutil.copytree(root, copy, ignore=shutil.ignore_patterns("target"))
    root = copy
    try:
        ran, took = run(["cargo", "blaze", "build", "--target", target], root,
                        f"cargo blaze build of {what}", env)
    finally:
        subprocess.run(["cargo", "blaze", "stop"], cwd=root, capture_output=True,
                       env={**os.environ, **env})
    if not re.search(r"^\s*Finished .*\[blaze\]", ran.stderr, re.M):
        raise SystemExit(f"FAIL: cargo blaze did not build {what} itself:\n{ran.stderr}")
    return root, took


def linked(ran, what):
    """How long ra-jit's link took, failing when it left something it could not compile."""
    said = re.search(r"^linked .* in (\S+), (\d+) functions in it blaze could not compile$",
                     ran.stderr, re.M)
    if not said:
        raise SystemExit(f"FAIL: {what}: ra-jit did not say it linked:\n{ran.stderr}")
    if said.group(2) != "0":
        raise SystemExit(f"FAIL: {what}: the module can reach {said.group(2)} functions ra-jit "
                         f"could not compile:\n{ran.stderr}")
    return said.group(1)


def nothing_of_rustc(ran, what):
    """That the standard library was ra-jit's own: compiled now or taken from the store."""
    said = re.findall(r"^standard library: (.*)$", ran.stderr, re.M)
    if not said:
        raise SystemExit(f"FAIL: {what}: ra-jit built no standard library of its own:\n{ran.stderr}")
    return "from the store" if said[0].startswith("from the store") else "compiled"


def kib(path):
    return f"{os.path.getsize(path) / 1024:.0f} KiB"


def main():
    if shutil.which(WASMTIME) is None:
        raise SystemExit(f"FAIL: no {WASMTIME} to run the modules with")
    work = scratch("ra-jit-allrajit-wasm-")
    store = os.path.join(work, "store")
    runtime = os.path.join(work, "run")
    os.makedirs(runtime, mode=0o700)
    passed = False
    lines = []
    try:
        for name, source, runs, panic in PROGRAMS:
            manifest = '[profile.dev]\npanic = "abort"\n' if panic == "abort" else ""
            root = package(work, name, manifest, "main.rs", source)
            run(["cargo", "build", "-q", "--target", WASI], root, f"cargo build of {name}")
            theirs = os.path.join(root, "target", WASI, "debug", f"{name}.wasm")
            expected = [outcome(theirs, args, root) for args in runs]
            ours = os.path.join(work, f"{name}.wasm")
            ran, took = ra_jit(root, name, os.path.join(work, f"{name}.o"), ours, WASI, store)
            std = nothing_of_rustc(ran, name)
            link_took = linked(ran, name)
            for args, want in zip(runs, expected):
                got = outcome(ours, args, root)
                if got != want:
                    raise SystemExit(f"FAIL: {name} {' '.join(args)} differs from cargo's:\n"
                                     f"cargo:  {want}\nra-jit: {got}")
            lines.append(f"{name} ({WASI}): built and linked in {took:.2f} s, the standard library "
                         f"{std}, the link {link_took}, {len(runs)} runs as cargo's; "
                         f"{kib(ours)}, cargo's {kib(theirs)}")
        cdylib = '[lib]\ncrate-type = ["cdylib"]\n'
        for name, what, source, config, calls in [
            ("shared", "the cdylib", LIBRARY, None, CALLS),
            ("guest", "the no_std SIMD guest", GUEST, GUEST_CONFIG, GUEST_CALLS),
        ]:
            root = package(work, name, cdylib, "lib.rs", source, config)
            run(["cargo", "build", "-q", "--target", UNKNOWN], root, f"cargo build of {what}")
            theirs = os.path.join(root, "target", UNKNOWN, "debug", f"{name}.wasm")
            expected = invoked(theirs, root, calls)
            if any(code != 0 for code, _ in expected):
                raise SystemExit(f"FAIL: cargo's build of {what} did not answer: {expected}")
            ours = os.path.join(work, f"{name}.wasm")
            ran, took = ra_jit(root, name, os.path.join(work, f"{name}.o"), ours, UNKNOWN, store,
                               "--shared")
            std = nothing_of_rustc(ran, what)
            link_took = linked(ran, what)
            got = invoked(ours, root, calls)
            if got != expected:
                raise SystemExit(f"FAIL: {what} differs from cargo's:\ncargo:  {expected}\nra-jit: {got}")
            lines.append(f"{what} ({UNKNOWN}): built and linked in {took:.2f} s, the standard "
                         f"library {std}, the link {link_took}, {len(calls)} calls as cargo's; "
                         f"{kib(ours)}, cargo's {kib(theirs)}")
            copy, took = cargo_blaze(root, what, store, runtime, UNKNOWN)
            served = os.path.join(copy, "target", UNKNOWN, "debug", f"{name}.wasm")
            got = invoked(served, root, calls)
            if got != expected:
                raise SystemExit(f"FAIL: cargo blaze's {what} differs from cargo's:\n"
                                 f"cargo:  {expected}\ncargo blaze: {got}")
            lines.append(f"{what} by `cargo blaze build`: {took:.2f} s, {len(calls)} calls as "
                         f"cargo's; {kib(served)}")
        print("\n".join(lines))
        print(f"PASS: {len(PROGRAMS)} {WASI} programs, a {UNKNOWN} cdylib and a no_std SIMD guest "
              "with nothing of rustc's, the last two by `cargo blaze build` too")
        passed = True
    finally:
        if passed:
            shutil.rmtree(work, ignore_errors=True)
        elif lines:
            print("\n".join(lines))


if __name__ == "__main__":
    main()
