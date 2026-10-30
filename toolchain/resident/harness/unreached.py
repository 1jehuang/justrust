#!/usr/bin/env python3
"""unreached.py: an edit that calls what the program did not call when it was linked is
spliced all the same, or the program is built in full. Linux and macOS.

usage: unreached.py    (RA_JIT_BIN names the ra-jit built; cargo-ra-jit is found next to it)

A patch calls the code of the executable by name, also code the executable's own did not
reach when it was linked: here `<u16 as Debug>::fmt` and the formatting it calls, which a
generic function of a library first calls for `u16` in an edit. blaze's link takes out what
nothing reaches (`-dead_strip` on macOS, `--gc-sections` on Linux), and a splice that does
not find what the patch calls declines ("... is not in the executable"): the worker then
builds the program in full and links it again, as the object alone does not have the edit.

The program is built with `cargo ra-jit build`, then edited. Every edit after the first one
(which writes the whole object again) has to be spliced or built in full, as the worker's
log says, and the program has to print what the source says.

Python 3, stdlib only.
"""
import glob
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
os.environ.setdefault("RUSTUP_TOOLCHAIN", open(os.path.join(HERE, "toolchain")).read().strip())
TIMEOUT = 900
FILES = {
    "Cargo.toml": '[workspace]\nmembers = ["app", "show"]\nresolver = "2"\n',
    "show/Cargo.toml": '[package]\nname = "ushow"\nversion = "0.1.0"\nedition = "2021"\n',
    "show/src/lib.rs": """pub fn show<T: Default + std::fmt::Debug>() -> String {
    format!("{:?}", T::default())
}
""",
    "app/Cargo.toml": '[package]\nname = "uapp"\nversion = "0.1.0"\nedition = "2021"\n\n'
                      '[dependencies]\nushow = { path = "../show" }\n',
    "app/src/main.rs": """fn main() {
    println!("{} {}", "a", ushow::show::<u8>());
}
""",
}
# (what the step is, [(old, new)] in app/src/main.rs, what the program prints after it)
STEPS = [
    ("the program's own text", [('"a"', '"b"')], "b 0"),
    ("an instance whose callee nothing called", [('"{} {}"', '"{} {} {}"'), ("<u8>()", "<u8>(), ushow::show::<u16>()")], "b 0 0"),
    ("and one more", [('"{} {} {}"', '"{} {} {} {}"'), ("<u16>()", "<u16>(), ushow::show::<i128>()")], "b 0 0 0"),
]


def build(ws, env, step):
    ran = subprocess.run(["cargo", "ra-jit", "build"], cwd=ws, env=env, capture_output=True, text=True,
                         timeout=TIMEOUT)
    said = ran.stdout + ran.stderr
    if ran.returncode != 0:
        raise SystemExit(f"{step}: the build failed:\n{said[-3000:]}")
    return said


def links(home):
    """The `link:` lines of the workers' logs, in order. A worker that was a spare keeps the
    spare's log."""
    lines = []
    for log in sorted(glob.glob(os.path.join(home, "*", "*", "log"))):
        lines += re.findall(r"^link: \S+ \((.*)\)$", open(log, errors="replace").read(), re.M)
    return lines


def full_builds(home):
    """How many builds the workers' logs say were in full, as a splice declined."""
    found = 0
    for log in glob.glob(os.path.join(home, "*", "*", "log")):
        found += len(re.findall(r"^the patch was not spliced into the program.*: built in full$",
                                open(log, errors="replace").read(), re.M))
    return found


def main():
    bin_dir = os.path.dirname(os.path.abspath(os.environ["RA_JIT_BIN"]))
    if not glob.glob(os.path.join(bin_dir, "cargo-ra-jit*")):
        raise SystemExit(f"no cargo-ra-jit beside {os.environ['RA_JIT_BIN']}")
    # The daemon's sockets are under its home, and a socket's path has to be short: macOS's
    # temporary directory is too long a start.
    root = tempfile.mkdtemp(prefix="ra-jit-unreached-", dir=None if os.name == "nt" else "/tmp")
    ws, home = os.path.join(root, "ws"), os.path.join(root, "h")
    for path, text in FILES.items():
        os.makedirs(os.path.dirname(os.path.join(ws, path)), exist_ok=True)
        open(os.path.join(ws, path), "w").write(text)
    env = dict(os.environ, PATH=f"{bin_dir}{os.pathsep}{os.environ['PATH']}", RA_JIT_HOME=home)
    for name in ("CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"):
        env.pop(name, None)
    program, source = os.path.join(ws, "target", "debug", "uapp"), os.path.join(ws, "app", "src", "main.rs")
    wrong = 0
    try:
        build(ws, env, "first build")
        for _ in range(240):
            if "[blaze]" in build(ws, env, "wait for the daemon"):
                break
            time.sleep(0.5)
        else:
            raise SystemExit(f"no build said `[blaze]`; the daemon's log is in {home}")
        for n, (what, edits, expected) in enumerate(STEPS):
            text = open(source).read()
            for old, new in edits:
                assert old in text, (what, old)
                text = text.replace(old, new, 1)
            open(source, "w").write(text)
            seen, full = len(links(home)), full_builds(home)
            said = build(ws, env, what)
            printed = subprocess.run([program], capture_output=True, text=True, timeout=60).stdout.strip()
            made = links(home)[seen:]
            how = made[-1] if made else "no link"
            verdict = "ok"
            if printed != expected:
                verdict = f"PRINTED {printed!r}, NOT {expected!r}"
            elif "[blaze]" not in said:
                verdict = "NOT SERVED BY THE DAEMON"
            elif n > 0 and not how.startswith("spliced") and full_builds(home) == full:
                verdict = "NOT SPLICED"
            if not how.startswith("spliced") and full_builds(home) > full:
                how = f"built in full; {how}"
            wrong += verdict != "ok"
            print(f"{what:42} {verdict:12} {how[:140]}", flush=True)
    finally:
        subprocess.run(["cargo", "ra-jit", "stop"], cwd=ws, env=env, capture_output=True, timeout=120)
    print(f"{len(STEPS) - wrong} of {len(STEPS)} right; under {root}")
    if wrong:
        sys.exit(1)
    shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
