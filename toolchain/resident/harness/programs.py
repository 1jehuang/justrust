#!/usr/bin/env python3
"""programs.py: two programs of one workspace built by one daemon.

The stress workspace gets a second binary. The daemon is started for the first; each step edits
files and then asks for one program or both by `ra-jit sync SOCKET --crate ...`. Every program
asked for is run and compared with what `cargo build` of the same source prints.
"""
import os, shlex, shutil, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import linkline
from difftest import Fixture, RA_JIT, sysroot_rlibs, RUNNER, TARGET_ARGS, built

TOOL_TOML = """[package]
name = "stool"
version = "0.1.0"
edition = "2021"

[dependencies]
sbase = { path = "../base" }
smid = { path = "../mid" }
"""
TOOL_MAIN = """use sbase::shapes::{total_area, Shape};
use smid::Tile;

fn weight(n: u32) -> u32 {
    n * 3
}

fn main() {
    let shapes = vec![Shape::Circle(3), Shape::Dot];
    println!("tool total {}", total_area(&shapes));
    let tile = Tile { side: 4, label: "tool".to_string() };
    println!("tool tile {}", total_area(&[tile]));
    println!("tool weight {}", weight(7));
}
"""
MID, TOOL, APP = "mid/src/lib.rs", "tool/src/main.rs", "app/src/main.rs"
AREA = "sbase::square!(self.side)"
# (what the step is, [(file, old, new)], programs asked for in order, whether cargo rejects it)
STEPS = [
    ("nothing edited, the second program asked for first time", [], ["stool"], False),
    ("nothing edited, both again", [], ["sapp", "stool", "stool"], False),
    ("code both use", [(MID, AREA, AREA + " + 1")], ["sapp", "stool"], False),
    ("code both use, the other asked first", [(MID, AREA + " + 1", AREA + " + 2")], ["stool", "sapp"], False),
    ("only the second program's code", [(TOOL, "n * 3", "n * 5")], ["stool", "sapp"], False),
    ("only the first program's code", [(APP, "items.iter().sum()", "items.iter().sum::<u32>() + 1")], ["stool", "sapp"], False),
    ("two edits before the second is asked", [(MID, AREA + " + 2", AREA + " + 3")], ["sapp"], False),
    ("the second catches up", [(TOOL, "n * 5", "n * 7")], ["stool", "sapp"], False),
    ("broken code both use", [(MID, AREA + " + 3", AREA + ' + "x"')], ["sapp", "stool", "sapp"], True),
    ("mended", [(MID, AREA + ' + "x"', AREA + " + 4")], ["stool", "sapp"], False),
    # rustc parses a parenthesized macro call with no `;` after it as an expression, so the outer
    # call's expansion, two calls with nothing between, does not parse (#154).
    ("a macro of several statements in the parentheses of another's call, which rustc rejects",
     [(MID, "        " + AREA + " + 4\n",
       "        macro_rules! twice {\n            ($($body:tt)*) => { $($body)* $($body)* };\n        }\n"
       "        let mut n = 0;\n        twice!(twice!(n += 1;));\n        " + AREA + " + 4 + n\n")],
     ["sapp", "stool"], True),
    ("the calls braced, which both accept", [(MID, "twice!(twice!(n += 1;));", "twice! { twice! { n += 1; } }")], ["stool", "sapp"], False),
    ("a function added to the second", [(TOOL, "fn main() {", "fn extra() -> u32 {\n    41\n}\n\nfn main() {\n    println!(\"extra {}\", extra());")], ["stool", "sapp"], False),
]


def splice(program):
    """Edits are added to the linked binary where that can be done; RA_JIT_SPLICE=0 links each in full."""
    return ["--binary", program["bin"]]


def main():
    ws = tempfile.mkdtemp(prefix="ra-jit-programs-")
    shutil.copytree(os.path.join(HERE, "stress"), ws, dirs_exist_ok=True, ignore=shutil.ignore_patterns("target"))
    os.makedirs(os.path.join(ws, "tool/src"))
    open(os.path.join(ws, "tool/Cargo.toml"), "w").write(TOOL_TOML)
    open(os.path.join(ws, TOOL), "w").write(TOOL_MAIN)
    manifest = os.path.join(ws, "Cargo.toml")
    members = open(manifest).read().replace('"app"]', '"app", "tool"]')
    open(manifest, "w").write(members)
    work = tempfile.mkdtemp(prefix="ra-jit-programs-out-")
    socket, log_path = os.path.join(work, "ra-jit.sock"), os.path.join(work, "daemon.log")
    reference = os.path.join(work, "reference")
    crates = ["sapp", "sbase", "smid", "stool"]
    programs = {}
    for name in ("sapp", "stool"):
        fx = Fixture(ws, crates, ["-p", name])
        fx.rustc_build()
        obj, patch, binary = (os.path.join(work, f"{name}.{n}") for n in ("o", "patch.o", "bin"))
        link = list(fx.link_args)
        link = linkline.with_output(linkline.ahead(link, [patch, obj], RA_JIT), binary)
        symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
        programs[name] = {"obj": obj, "patch": patch, "bin": binary, "link": linkline.join(link), "symbols": symbols}
    first = programs["sapp"]
    out = open(log_path, "a")
    daemon = subprocess.Popen(
        [RA_JIT, ws, *TARGET_ARGS, *[x for c in crates for x in ("--crate", c)], "--out", first["obj"], "--socket", socket,
         "--patch", first["patch"], "--link-cmd", first["link"], *splice(first), *first["symbols"]], stderr=out, stdout=out)
    while "watching" not in open(log_path).read():
        if daemon.poll() is not None:
            raise SystemExit(f"the daemon did not start: {log_path}")
        time.sleep(0.05)

    def run(path):
        try:
            ran = subprocess.run([*RUNNER, path], capture_output=True, text=True, timeout=30)
        except Exception as error:
            return f"did not run: {error}"
        return ran.stdout + (f"[exit {ran.returncode}]" if ran.returncode else "")

    wrong = asked = 0
    for what, edits, order, broken in STEPS:
        for path, old, new in edits:
            text = open(os.path.join(ws, path)).read()
            assert old in text, (what, old)
            open(os.path.join(ws, path), "w").write(text.replace(old, new, 1))
        time.sleep(0.02)
        print(what, flush=True)
        for name in order:
            program = programs[name]
            began = time.time()
            reply = subprocess.run(
                [RA_JIT, "sync", socket, "--crate", name, "--out", program["obj"], "--patch", program["patch"],
                 "--link-cmd", program["link"], *splice(program), *program["symbols"]], capture_output=True, text=True, timeout=120)
            took = (time.time() - began) * 1000
            reply = (reply.stdout + reply.stderr).strip()
            cargo = subprocess.run(["cargo", "build", "-q", "-p", name, *TARGET_ARGS], cwd=ws, capture_output=True, text=True,
                                   env={**os.environ, "CARGO_TARGET_DIR": reference})
            assert (cargo.returncode != 0) == broken, cargo.stderr[-2000:]
            if daemon.poll() is not None:
                verdict = "DAEMON DIED"
            elif broken:
                verdict = "refused" if not reply.startswith("ok") else "ANSWERED OK"
            elif not reply.startswith("ok"):
                verdict = "REFUSED"
            else:
                verdict = "same" if run(program["bin"]) == run(built(reference, name)) else "DIFFERENT OUTPUT"
            asked += 1
            wrong += verdict.isupper()
            print(f"    {name:6} {verdict:16} {took:6.0f} ms  {reply[:90]}", flush=True)
            if daemon.poll() is not None:
                raise SystemExit(f"the daemon died: {log_path}")
    daemon.kill()
    print(f"{asked - wrong} of {asked} right; log {log_path}")
    sys.exit(1 if wrong else 0)


if __name__ == "__main__":
    main()
