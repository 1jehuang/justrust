#!/usr/bin/env python3
"""edittimes.py: how long it is from an edit to the program running, linked in full and spliced.

usage: edittimes.py WORKSPACE PACKAGE CRATE[,CRATE...] FILE FIND REPLACE [--runs N] [-- RUN_ARG...]

The workspace is copied and one daemon is started on it for each way of linking: without
`--splice`, where every edit runs the linker, and with it. Each run of each daemon swaps FIND
and REPLACE in FILE, asks for a build with `ra-jit sync` and runs the program. Printed for
every run: the time `ra-jit sync` took (compiling, and linking or splicing), what of it the
link or the splice was by the daemon's log, and the time the program then took to run. Every
run is printed, so that nothing is a median of unseen numbers.

With RA_JIT_TARGET the program runs under an emulator: its run time is the emulator's, not
that of the target's hardware. The daemon and the linker are the host's own programs.
"""
import os, re, shlex, shutil, statistics, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
os.environ.setdefault("RUSTUP_TOOLCHAIN", open(os.path.join(HERE, "toolchain")).read().strip())
sys.path.insert(0, HERE)
from difftest import Fixture, RA_JIT, RUNNER, sysroot_rlibs

UNITS = {"µs": 0.001, "ms": 1, "s": 1000}


def measure(ws, package, crates, path, pair, runs, run_args, spliced):
    work = tempfile.mkdtemp(prefix="ra-jit-edittimes-out-")
    socket, log_path = os.path.join(work, "ra-jit.sock"), os.path.join(work, "daemon.log")
    fx = Fixture(ws, crates, ["-p", package])
    fx.rustc_build()
    obj, patch, binary = (os.path.join(work, f"{package}.{n}") for n in ("o", "patch.o", "bin"))
    link = list(fx.link_args)
    first = next(i for i, a in enumerate(link) if a.endswith((".rlib", ".rcgu.o")))
    link[first:first] = [patch, obj, "-Wl,--allow-multiple-definition"]
    link[link.index("-o") + 1] = binary
    symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
    ask = [*(x for c in crates for x in ("--crate", c)), "--out", obj, "--patch", patch, "--link-cmd", shlex.join(link),
           *(["--splice", binary] if spliced else []), *symbols]
    out = open(log_path, "a")
    began = time.perf_counter()
    daemon = subprocess.Popen([RA_JIT, ws, "--socket", socket, *ask], stderr=out, stdout=out)
    while "watching" not in open(log_path).read():
        if daemon.poll() is not None:
            raise SystemExit(f"the daemon did not start: {log_path}")
        time.sleep(0.05)
    print(f"  first build {time.perf_counter() - began:.1f} s, binary {os.path.getsize(binary) >> 20} MB", flush=True)
    rows = []
    for at in range(runs):
        old, new = pair if at % 2 == 0 else pair[::-1]
        text = open(os.path.join(ws, path)).read()
        assert old in text, old
        open(os.path.join(ws, path), "w").write(text.replace(old, new, 1))
        seen = len(open(log_path).read())
        began = time.perf_counter()
        reply = subprocess.run([RA_JIT, "sync", socket], capture_output=True, text=True, timeout=600)
        synced = time.perf_counter()
        ran = subprocess.run([*RUNNER, binary, *run_args], capture_output=True, timeout=120, stdin=subprocess.DEVNULL)
        done = time.perf_counter()
        assert reply.stdout.startswith("ok patch"), reply.stdout + reply.stderr
        linked = re.findall(r"^link: ([\d.]+)(µs|ms|s) \((.*)\)$", open(log_path).read()[seen:], re.M)[-1]
        how = "spliced" if linked[2].startswith("spliced") else "full link"
        assert spliced == (how == "spliced"), linked[2]
        row = ((synced - began) * 1000, float(linked[0]) * UNITS[linked[1]], (done - synced) * 1000)
        rows.append(row)
        print(f"  {how:9} sync {row[0]:7.1f} ms  of which link {row[1]:8.2f} ms  run {row[2]:7.1f} ms  exit {ran.returncode}", flush=True)
    daemon.kill()
    # The first is the patch that makes the binary the linker's own again.
    rows = rows[1:] or rows
    median = [statistics.median(col) for col in zip(*rows)]
    print(f"  median of {len(rows)}: sync {median[0]:.1f} ms, link {median[1]:.2f} ms, run {median[2]:.1f} ms, "
          f"edit to running {median[0] + median[2]:.1f} ms", flush=True)


def main():
    args = sys.argv[1:]
    run_args = args[args.index("--") + 1:] if "--" in args else []
    args = args[:args.index("--")] if "--" in args else args
    runs = int(args[args.index("--runs") + 1]) if "--runs" in args else 11
    source, package, crates, path, find, replace = args[:6]
    ws = tempfile.mkdtemp(prefix="ra-jit-edittimes-")
    shutil.copytree(source, ws, dirs_exist_ok=True, ignore=shutil.ignore_patterns("target", ".git"))
    for spliced in (False, True):
        print("spliced" if spliced else "linked in full", flush=True)
        measure(ws, package, crates.split(","), path, (find, replace), runs, run_args, spliced)


if __name__ == "__main__":
    main()
