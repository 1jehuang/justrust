#!/usr/bin/env python3
"""editloop.py: how long an edit takes to reach a program that can be run.

    editloop.py WORKSPACE CRATE FILE OLD NEW [--rounds N] [--after CMD] [--run]

Builds CRATE with cargo (timed, from whatever its target directory already holds), starts the
daemon on it (timed up to the first link), and then N times replaces OLD by NEW in FILE and
back, asking the daemon for the program after each. For each edit it prints the time of the
daemon's answer, which includes the link or the splice, the time of `--after` (a command run
on the linked program with `{}` standing for its path and `{n}` for the number of the edit,
wasm-bindgen for one), and with `--run`
that of running it, whose output must change with each edit. `RA_JIT_TARGET` and `RA_JIT_SPLICE`
mean what they do to the other harnesses.
"""
import argparse, os, shlex, statistics, subprocess, sys, tempfile, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from difftest import Fixture, RA_JIT, sysroot_rlibs, AHEAD, TARGET_ARGS, command


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("workspace"), parser.add_argument("crate"), parser.add_argument("file")
    parser.add_argument("old"), parser.add_argument("new")
    parser.add_argument("--rounds", type=int, default=5)
    parser.add_argument("--after")
    parser.add_argument("--run", action="store_true")
    args = parser.parse_args()
    ws, source = os.path.abspath(args.workspace), os.path.join(args.workspace, args.file)
    work = tempfile.mkdtemp(prefix="ra-jit-editloop-")
    socket, log_path = os.path.join(work, "ra-jit.sock"), os.path.join(work, "daemon.log")
    obj, patch, binary = (os.path.join(work, f"program.{n}") for n in ("o", "patch.o", "bin"))

    fx = Fixture(ws, [args.crate], ["-p", args.crate])
    began = time.time()
    fx.rustc_build()
    print(f"cargo build: {time.time() - began:.2f} s")
    link = list(fx.link_args)
    first = next(i for i, a in enumerate(link) if a.endswith((".rlib", ".rcgu.o")))
    link[first:first] = [patch, obj, *AHEAD]
    link[link.index("-o") + 1] = binary
    if len(shlex.join(link)) > 100_000:
        # One argument holds the whole line, and the kernel takes no argument that long.
        flavor = 3 if link[1] == "-flavor" else 1
        response = os.path.join(work, "link.args")
        open(response, "w").write("\n".join(shlex.quote(a) for a in link[flavor:]))
        link[flavor:] = ["@" + response]
    symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
    target = ["--out", obj, "--patch", patch, "--link-cmd", shlex.join(link),
              *(["--splice", binary] if os.environ.get("RA_JIT_SPLICE") else []), *symbols]
    out = open(log_path, "a")
    began = time.time()
    daemon = subprocess.Popen([RA_JIT, ws, *TARGET_ARGS, "--crate", args.crate, "--socket", socket, *target],
                              stderr=out, stdout=out)
    while "watching" not in open(log_path).read():
        if daemon.poll() is not None:
            raise SystemExit(f"the daemon did not start: {log_path}")
        time.sleep(0.01)
    print(f"daemon to first link: {time.time() - began:.2f} s; program {os.path.getsize(binary)} bytes")

    def timed(argv):
        began = time.time()
        ran = subprocess.run(argv, capture_output=True, text=True, timeout=300)
        return (time.time() - began) * 1000, ran

    original = open(source).read()
    assert args.old in original, args.old
    columns, outputs, wrong = {"sync": [], "after": [], "run": []}, [], 0
    try:
        for step in range(args.rounds * 2):
            text = original.replace(args.old, args.new, 1) if step % 2 == 0 else original
            open(source, "w").write(text)
            time.sleep(0.02)
            took, reply = timed([RA_JIT, "sync", socket, "--crate", args.crate, *target])
            reply = (reply.stdout + reply.stderr).strip()
            columns["sync"].append(took)
            line = f"  edit {step:2}  sync {took:6.0f} ms"
            if args.after:
                took, ran = timed(shlex.split(args.after.replace("{}", binary).replace("{n}", str(step))))
                columns["after"].append(took)
                line += f"  after {took:6.0f} ms" + ("" if ran.returncode == 0 else " FAILED")
                wrong += ran.returncode != 0
            if args.run:
                took, ran = timed(command(binary))
                columns["run"].append(took)
                outputs.append(ran.stdout)
                changed = len(outputs) < 2 or outputs[-1] != outputs[-2]
                agrees = len(outputs) < 3 or outputs[-1] == outputs[-3]
                line += f"  run {took:6.0f} ms" + ("" if changed and agrees else " OUTPUT WRONG")
                wrong += not (changed and agrees)
            wrong += not reply.startswith("ok")
            print(f"{line}  {reply[:70]}", flush=True)
    finally:
        open(source, "w").write(original)
        daemon.kill()
    for name, times in columns.items():
        if times:
            print(f"{name}: median {statistics.median(times):.0f} ms, min {min(times):.0f}, max {max(times):.0f}")
    links = [l.strip() for l in open(log_path) if l.startswith("link:")]
    print("the daemon's links:", "; ".join(links[-4:]))
    print(f"log {log_path}")
    sys.exit(1 if wrong else 0)


if __name__ == "__main__":
    main()
