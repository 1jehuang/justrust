#!/usr/bin/env python3
"""backends.py: one program compiled by each code producer, and what each made of it.

usage: backends.py WORKSPACE PACKAGE CRATE[,CRATE...] [--bin NAME] [--project DIR]
                   [--levels 0,2] [--runs N] [--kernels] [--out FILE] [-- ARGS]

Builds the program with cargo, then has ra-jit compile the crates once with Cranelift and once
with LLVM at each level, links each object ahead of rustc's and compares what the program prints
with cargo's. Reports, for each producer: how many functions it compiled and how many LLVM left
to Cranelift and why, the time spent producing code (summed over threads) and per function, the
size of the code, and ra-jit's wall time and peak memory. With `--kernels` the program is
harness/corpus/kernels: each kernel is then run `--runs` times and its best time reported, next
to cargo's debug and release builds.
"""
import os, re, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from difftest import Fixture, RA_JIT, sysroot_rlibs


class Compiled:
    """One run of ra-jit over the crates with one producer."""

    def __init__(self, fx, name, env, project):
        self.name = name
        obj = os.path.join(fx.work, name.replace(" ", "") + ".o")
        symbols = [x for rlib in fx.rlibs + sysroot_rlibs() for x in ("--symbols", rlib)]
        crates = [x for c in fx.crates for x in ("--crate", c)]
        start = time.monotonic()
        child = subprocess.Popen([RA_JIT, project, *crates, "--out", obj, *symbols],
                                 stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, text=True, encoding="utf-8",
                                 env={**os.environ, "RA_JIT_STATS": "1", **env})
        self.stderr = child.stderr.read()
        if hasattr(os, "wait4"):
            _, status, usage = os.wait4(child.pid, 0)
            # `ru_maxrss` is in bytes on macOS and in kilobytes on Linux.
            unit = 1 << 20 if sys.platform == "darwin" else 1 << 10
            self.rss_mb = usage.ru_maxrss // unit
        else:
            # Windows has no `wait4`: the peak is not measured there.
            status, self.rss_mb = child.wait(), 0
        self.wall = time.monotonic() - start
        if status != 0:
            raise SystemExit(f"ra-jit failed with {name}:\n{self.stderr[-3000:]}")
        open(os.path.join(fx.work, name.replace(" ", "") + ".stderr"), "w", encoding="utf-8").write(self.stderr)
        # A program ra-jit cannot link with any producer still says what each producer costs.
        try:
            fx.link([obj])
            self.binary = os.path.join(fx.work, "bin-" + name.replace(" ", ""))
            os.rename(fx.binary, self.binary)
        except SystemExit as failure:
            self.binary, self.link_failure = None, str(failure).splitlines()[1][:200]
        self.produced = {who: (int(n), int(size), seconds(took)) for who, n, size, took in re.findall(
            r"^backend: (\w+) compiled (\d+) functions, (\d+) bytes, (\S+) ", self.stderr, re.M)}
        self.phases = re.search(r"^backend: llvm compiled .*: (.*)$", self.stderr, re.M)
        self.fallbacks = re.findall(r"^  (\d+)\t(.*)$", self.stderr, re.M)
        self.large = re.search(r"^backend: llvm compiled (\d+) large functions", self.stderr, re.M)
        self.cold = re.search(r"^cold: .*$", self.stderr, re.M)

    def row(self):
        llvm, cranelift = self.produced.get("llvm"), self.produced.get("cranelift")
        mine = llvm if self.name.startswith("llvm") else cranelift
        functions = sum(it[0] for it in self.produced.values())
        size = sum(it[1] for it in self.produced.values())
        share = f"{100 * mine[0] / functions:.2f}%" if mine else "n/a"
        each = f"{1e6 * mine[2] / mine[0]:.0f}" if mine else "n/a"
        return (f"| {self.name} | {functions} | {share} | {mine[2]:.1f} | {each} | {size} "
                f"| {self.wall:.1f} | {self.rss_mb} |")


def seconds(text):
    number, unit = re.match(r"([\d.]+)(\D+)", text).groups()
    return float(number) * {"s": 1, "ms": 1e-3, "µs": 1e-6, "ns": 1e-9}[unit]


def run(binary, args):
    """What a program does. A usage line names the program by its file, which differs between builds."""
    done = subprocess.run([binary, *args], capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=1800, stdin=subprocess.DEVNULL)
    return done.returncode, done.stdout.replace(os.path.basename(binary), "PROGRAM"), done.stderr


def kernel_times(binary, runs):
    """The best of `runs` for each kernel, in milliseconds, and what the program printed."""
    best, printed = {}, None
    for _ in range(runs):
        status, printed, stderr = run(binary, [])
        assert status == 0, stderr
        for name, ms in (line.split() for line in stderr.splitlines()):
            best[name] = min(best.get(name, 1 << 62), int(ms))
    return best, printed


def main():
    argv = sys.argv[1:]
    run_args = argv[argv.index("--") + 1:] if "--" in argv else []
    argv = argv[: argv.index("--")] if "--" in argv else argv
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    workspace, package, crates = os.path.abspath(argv[0]), argv[1], argv[2]
    levels, runs = [it for it in opt("--levels", "0,2").split(",") if it], int(opt("--runs", "3"))
    target = ["-p", package] + (["--bin", opt("--bin")] if opt("--bin") else [])
    fx = Fixture(workspace, crates.split(","), target)
    start = time.monotonic()
    fx.rustc_build()
    lines = [f"cargo build: {time.monotonic() - start:.1f} s (whatever was not built already), work in {fx.work}", ""]
    expected = run(fx.reference, run_args)
    project = os.path.join(workspace, opt("--project", ""))
    producers = [("cranelift", {"RA_JIT_BACKEND": "cranelift"})]
    # A level with an `f` after it is that level's passes with LLVM's fast code selection.
    for level in levels:
        fast = level.endswith("f")
        env = {"RA_JIT_BACKEND": "llvm", "RA_JIT_OPT": level.rstrip("f"), "RA_JIT_EMIT": "fast" if fast else "optimized"}
        producers.append((f"llvm -O{level.rstrip('f')}" + (" fast emit" if fast else ""), env))
    built = []
    for name, env in producers:
        compiled = Compiled(fx, name, env, project)
        built.append(compiled)
        print(f"{name}: {compiled.wall:.1f} s", file=sys.stderr)
    lines += ["| producer | functions | share by this producer | producing code, s (all threads) | µs per function | code bytes | ra-jit wall, s | ra-jit peak RSS, MB |",
              "|---|---:|---:|---:|---:|---:|---:|---:|"]
    lines += [it.row() for it in built]
    lines.append("")
    for it in built:
        if it.phases:
            lines.append(f"- {it.name} phases (all threads): {it.phases[1]}")
        if it.large:
            lines.append(f"- {it.name} compiled {it.large[1]} large functions at -O0")
        for count, reason in it.fallbacks:
            lines.append(f"- {it.name} left {count} to Cranelift: {reason}")
        if it.cold:
            lines.append(f"- {it.name}: {it.cold[0]}")
    lines.append("")
    if "--kernels" in argv:
        release = os.path.join(fx.work, "release")
        subprocess.run(["cargo", "build", "--release", *target], cwd=workspace, check=True, capture_output=True,
                       env={**os.environ, "CARGO_TARGET_DIR": release})
        columns = [("cargo debug", fx.reference), *((it.name, it.binary) for it in built),
                   ("cargo release", os.path.join(release, "release", package))]
        times = {}
        for name, binary in columns:
            times[name], printed = kernel_times(binary, runs)
            assert printed == expected[1], f"{name} prints something else"
        lines += [f"Best of {runs} runs, ms; every build prints the same checksums.", "",
                  "| kernel | " + " | ".join(name for name, _ in columns) + " |", "|---|" + "---:|" * len(columns)]
        for kernel in times["cargo debug"]:
            lines.append(f"| {kernel} | " + " | ".join(str(times[name][kernel]) for name, _ in columns) + " |")
    else:
        for it in built:
            if it.binary is None:
                lines.append(f"- {it.name}: does not link: {it.link_failure}")
                continue
            same = run(it.binary, run_args)[:2] == expected[:2]
            lines.append(f"- {it.name}: the program prints {'the same as' if same else 'SOMETHING ELSE than'} cargo's")
    text = "\n".join(lines) + "\n"
    sys.stdout.reconfigure(encoding="utf-8")
    print(text)
    if opt("--out"):
        open(opt("--out"), "w", encoding="utf-8").write(text)


if __name__ == "__main__":
    main()
