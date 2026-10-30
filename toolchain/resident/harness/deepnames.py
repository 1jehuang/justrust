#!/usr/bin/env python3
"""deepnames.py: the name a store entry is found by stays short however deep closures nest.

A closure that wraps a closure has a type that holds the one it wraps twice, once among the
generic arguments of the function it is written in and once among what it captures. Written
out in full, the name of the outermost of `DEPTH` doubles with each level: helix's parser
combinators made one of 665 MB, and the store held 14.6 GB of copies of a few such names.

A daemon builds a program of closures nested `DEPTH` deep with an empty store and ends; a
second one on the same store builds it again:

  - both programs have to print what cargo's does;
  - the store has to stay under `MOST` bytes;
  - the second daemon has to find in the store what the first put there.

    python3 deepnames.py
"""
import os, re, shutil, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import linkline
from difftest import Fixture, RA_JIT, sysroot_rlibs, TARGET_ARGS, built
from storelib import run, scratch

DEPTH = 20
MOST = 64 << 20
AGAIN = "compiling what the store had of the checkout"
LINK = re.compile(r"^link: ", re.M)

TOML = """[package]
name = "deep"
version = "0.1.0"
edition = "2024"

[workspace]
"""
MAIN = """#[inline(never)]
fn wrap<P: Fn(u64) -> u64>(p: P, k: u64) -> impl Fn(u64) -> u64 {
    move |x| p(x).wrapping_mul(3).wrapping_add(k)
}

fn main() {
    let f = |x: u64| x + 1;
%s    println!("{}", f(std::env::args().count() as u64));
}
"""


def ran(binary):
    """What the program prints. Just linked, it can still be open in a process that forked as
    it was written (Linux says ETXTBSY), for a moment."""
    deadline = time.monotonic() + 5
    while "Text file busy" in (got := run(binary)) and time.monotonic() < deadline:
        time.sleep(0.1)
    return got


def size(store):
    return sum(os.path.getsize(os.path.join(root, name)) for root, _, names in os.walk(store) for name in names)


def main():
    work, store = scratch("ra-jit-deepnames-"), scratch("ra-jit-deepnames-store-")
    ws = os.path.join(work, "deep")
    os.makedirs(os.path.join(ws, "src"))
    levels = "".join(f"    let f = wrap(f, {k});\n" for k in range(DEPTH))
    for path, text in (("Cargo.toml", TOML), ("src/main.rs", MAIN % levels)):
        open(os.path.join(ws, path), "w").write(text)
    fx = Fixture(ws, ["deep"], ["-p", "deep"])
    fx.rustc_build()
    obj, patch, binary = (os.path.join(work, f"deep.{n}") for n in ("o", "patch.o", "bin"))
    link = linkline.join(linkline.with_output(linkline.ahead(list(fx.link_args), [patch, obj], RA_JIT), binary))
    symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
    ask = ["--crate", "deep", "--out", obj, "--patch", patch, "--link-cmd", link, "--binary", binary, *symbols]
    reference = os.path.join(work, "reference")
    cargo = subprocess.run(["cargo", "build", "-q", *TARGET_ARGS], cwd=ws, capture_output=True, text=True,
                           env={**os.environ, "CARGO_TARGET_DIR": reference})
    assert cargo.returncode == 0, cargo.stderr[-2000:]
    want = run(built(reference, "deep"))
    passed = False
    try:
        logs = []
        for name in ("first", "second"):
            log_path = os.path.join(work, f"{name}.log")
            socket = os.path.join(work, f"{name}.sock")
            began = time.monotonic()
            with open(log_path, "w") as out:
                daemon = subprocess.Popen([RA_JIT, ws, *TARGET_ARGS, "--socket", socket, *ask],
                                          stderr=out, stdout=out, env={**os.environ, "RA_JIT_CACHE": store})
            try:
                while "watching" not in open(log_path).read():
                    if daemon.poll() is not None:
                        raise SystemExit(f"FAIL: the {name} daemon did not start: {log_path}")
                    time.sleep(0.05)
                took = time.monotonic() - began
                reply = subprocess.run([RA_JIT, "sync", socket, *ask], capture_output=True, text=True, timeout=300)
                # What came from the store of the checkout is compiled again, and linked, by itself,
                # and the reply can come before the daemon says it does.
                asked, deadline = time.monotonic(), time.monotonic() + 120
                while True:
                    text = open(log_path).read()
                    if AGAIN in text and LINK.search(text.split(AGAIN, 1)[1]):
                        break
                    if AGAIN not in text and time.monotonic() > asked + 3:
                        break
                    if time.monotonic() > deadline:
                        raise SystemExit(f"FAIL: the {name} daemon's build by itself is not over: {log_path}")
                    time.sleep(0.1)
                got = ran(binary)
                if got != want:
                    raise SystemExit(f"FAIL: the {name} daemon's program prints\n{got}\nand cargo's\n{want}\n"
                                     f"reply: {(reply.stdout + reply.stderr).strip()}\nlog: {log_path}")
            finally:
                daemon.kill()
                daemon.wait()
            logs.append(open(log_path).read())
            print(f"{name} daemon: built in {took:.1f}s, store {size(store) / 1e6:.1f} MB", flush=True)
        if size(store) > MOST:
            raise SystemExit(f"FAIL: closures {DEPTH} deep made a store of {size(store)} bytes, more than {MOST}")
        found = [int(n) for n in re.findall(r"^entries: (\d+) found", logs[1], re.M)]
        if not found or found[0] == 0:
            raise SystemExit(f"FAIL: the second daemon found nothing in the store: {work}/second.log")
        print(f"PASS: closures {DEPTH} deep, {found[0]} entries found again")
        passed = True
    finally:
        if passed:
            shutil.rmtree(work, ignore_errors=True)
            shutil.rmtree(store, ignore_errors=True)


if __name__ == "__main__":
    main()
