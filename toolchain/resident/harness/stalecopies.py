#!/usr/bin/env python3
"""stalecopies.py: plain `cargo build` through `ra-jit enable` never runs code of before an edit
that rustc's copies still have (#386).

    stalecopies.py [--work DIR] [SCENARIO...]

`allocator`: a `#[global_allocator]` at opt-level 1, where rustc's allocator entries have its
`alloc` inlined: each edit of `alloc` is built by the daemon and runs. `restart`: the daemon
stops, `main` is edited, and the daemon starts again before cargo built the edit; `main` is left
to rustc (an `asm!` with a `sym` operand), so rustc's copy of it is of the text before, and the
build is cargo's.
Every edit's program has to print what its source says. Exits 1 when one does not.
"""
import os, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
TOOLCHAIN = open(os.path.join(HERE, "toolchain")).read().strip()
BIN = os.path.dirname(os.path.abspath(os.environ.get("RA_JIT_BIN") or os.path.join(HERE, "../target/release/ra-jit")))

ALLOCATOR = """\
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

#[unsafe(no_mangle)]
pub static LAST: AtomicUsize = AtomicUsize::new(0);

struct Last;

unsafe impl GlobalAlloc for Last {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LAST.store(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static A: Last = Last;

fn main() {
    let v: Vec<u8> = vec![1; std::env::args().count() + 3];
    println!("{} {}", LAST.load(Ordering::Relaxed), v.len());
}
"""

LEFT = """\
extern "C" fn mark() {}

fn main() {
    // An `asm!` with a `sym` operand is not compiled by ra-jit: `main` is left to rustc.
    unsafe { core::arch::asm!("/* {0} */", sym mark) };
    let k = 1usize;
    println!("{} {}", k, std::env::args().count() + 3);
}
"""

# name: (source, the text of the value `k` is in)
SCENARIOS = {"allocator": (ALLOCATOR, "store({k},"), "restart": (LEFT, "let k = {k}usize")}


class Workspace:
    def __init__(self, work, source):
        shutil.rmtree(work, ignore_errors=True)
        self.root = work
        self.ws = os.path.join(work, "ws")
        os.makedirs(os.path.join(self.ws, "app/src"))
        os.makedirs(os.path.join(work, "cargo-home"))
        self.write("Cargo.toml", '[workspace]\nmembers = ["app"]\nresolver = "3"\n\n[profile.dev]\nopt-level = 1\n')
        self.write("app/Cargo.toml", '[package]\nname = "sapp"\nversion = "0.1.0"\nedition = "2024"\n')
        self.write("app/src/main.rs", source)
        self.env = dict(os.environ, PATH=f"{BIN}:{os.environ['PATH']}", RUSTUP_TOOLCHAIN=TOOLCHAIN,
                        RA_JIT_HOME=os.path.join(work, "home"), RA_JIT_CACHE=os.path.join(work, "store"),
                        CARGO_HOME=os.path.join(work, "cargo-home"), CARGO_INCREMENTAL="0", RUST_BACKTRACE="0")
        for name in ("CARGO_TARGET_DIR", "RUSTFLAGS", "RUSTC_WORKSPACE_WRAPPER", "RA_JIT_RUSTC_BASE"):
            self.env.pop(name, None)

    def write(self, path, text):
        open(os.path.join(self.ws, path), "w").write(text)

    def run(self, *cmd):
        return subprocess.run(list(cmd), cwd=self.ws, env=self.env, capture_output=True, text=True, timeout=600)

    def edit(self, pattern, k):
        path = os.path.join(self.ws, "app/src/main.rs")
        text = open(path).read()
        old, new = pattern.format(k=k - 1), pattern.format(k=k)
        assert text.count(old) == 1, f"{old!r} is not once in main.rs"
        open(path, "w").write(text.replace(old, new))

    def last_build(self):
        path = os.path.join(self.root, "home", "plain-cargo.log")
        lines = open(path).read().strip().splitlines() if os.path.exists(path) else []
        return lines[-1] if lines else ""

    def build(self):
        r = self.run("cargo", "build")
        if r.returncode:
            raise RuntimeError(f"cargo build failed: {r.stderr[-1500:]}")
        return self.last_build()

    def printed(self):
        return self.run(os.path.join(self.ws, "target/debug/sapp")).stdout.strip()

    def warm(self):
        """Builds until the daemon serves one."""
        for _ in range(240):
            os.utime(os.path.join(self.ws, "app/src/main.rs"))
            if self.build().startswith("daemon"):
                return
            time.sleep(0.5)
        raise RuntimeError("the daemon served no build")


def scenario(name, work):
    source, pattern = SCENARIOS[name]
    ws = Workspace(work, source)
    failed = []
    r = ws.run(os.path.join(BIN, "ra-jit"), "enable")
    if r.returncode:
        return [f"{name}: ra-jit enable failed: {r.stderr[-500:]}"]
    try:
        ws.build()
        ws.warm()
        for k in (2, 3, 4):
            if name == "restart":
                ws.run(os.path.join(BIN, "ra-jit"), "stop")
            ws.edit(pattern, k)
            how = ws.build()
            printed = ws.printed()
            right = printed.startswith(f"{k} ")
            print(f"  {name} k={k}: printed {printed!r} ({how[:160]})", "" if right else "STALE", flush=True)
            if not right:
                failed.append(f"{name} k={k}: printed {printed!r}, built by {how[:200]}")
            if name == "allocator" and not how.startswith("daemon"):
                failed.append(f"{name} k={k}: not the daemon's: {how[:200]}")
            if name == "restart":
                ws.warm()
    except Exception as e:
        failed.append(f"{name}: {e}")
    finally:
        ws.run(os.path.join(BIN, "ra-jit"), "stop")
        ws.run(os.path.join(BIN, "ra-jit"), "disable")
    return failed


def main():
    work = "/tmp/stalecopies"
    if "--work" in sys.argv:
        at = sys.argv.index("--work")
        work = sys.argv[at + 1]
        del sys.argv[at:at + 2]
    names = sys.argv[1:] or list(SCENARIOS)
    failed = []
    for name in names:
        print(f"{name}:", flush=True)
        failed += scenario(name, os.path.join(os.path.realpath(work), name))
    print("\n" + ("\n".join(failed) if failed else f"all {len(names)} scenarios as expected"))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
