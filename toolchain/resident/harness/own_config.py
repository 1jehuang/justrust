#!/usr/bin/env python3
"""own_config.py: `cargo blaze config` against `cargo config`. cargo is the oracle; blaze never
runs it outside this harness.

usage: own_config.py [--blaze BIN] [--out FILE]

A made-up workspace has a `.cargo/config.toml` and one in a member below it, the second
enabling the unstable options, and each case runs `cargo config ARGS` and, as a user runs
blaze's, `cargo blaze config ARGS` with the `cargo-blaze` beside BIN (`ra-jit`, default
`target/release/ra-jit`) first on the path, in the workspace or the member, with `CARGO_BUILD_JOBS` and `CARGO_ENV_X` set, comparing the exit status,
standard output and standard error. A case may first change the files: a configuration that
is not TOML.
"""
import argparse, json, os, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

TOP = """\
[build]
jobs = 3
rustflags = ["-Cx", "it's", 'a "b"']
[alias]
q = "check"
[env]
X = { value = "1", relative = true }
Y = "two words"
[target.'cfg(unix)']
runner = "run me"
"""
MEMBER = """\
[build]
rustflags = ["-Cy"]
[unstable]
unstable-options = true
"""

Z = "-Zunstable-options"


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def not_toml(top):
    write(os.path.join(top, ".cargo", "config.toml"), "[build\n")


def member(top):
    return "a"


CASES = [
    ("get", None),
    ("get build.jobs", None),
    ("get build.jobs", member),
    (f"get {Z}", None),
    (f"{Z} get", None),
    (f"get -Z unstable-options build", None),
    (f"get {Z} build.jobs --show-origin", None),
    (f"get {Z} build.rustflags --show-origin", None),
    (f"get {Z} env", None),
    (f"get {Z} env --show-origin", None),
    (f"get {Z} alias.q", None),
    (f"get {Z} target", None),
    (f"get {Z} --format json", None),
    (f"get {Z} --format json build", None),
    (f"get {Z} --format json-value build.rustflags", None),
    (f"get {Z} --format json-value build.jobs", None),
    (f"get {Z} --format json --show-origin", None),
    (f"get {Z} --format yaml", None),
    (f"get {Z} --merged no", None),
    (f"get {Z} --merged no build", None),
    (f"get {Z} --merged=no build.rustflags --show-origin", None),
    (f"get {Z} --merged no --format json", None),
    (f"get {Z} --merged maybe", None),
    (f"get {Z} build.jobs.x", None),
    (f"get {Z} --merged no build.jobs.x", None),
    (f"get {Z} nothing.here", None),
    (f"get {Z} --config build.jobs=9 build.jobs --show-origin", None),
    (f"get {Z} --config build.jobs=9 --merged no build", None),
    (f"--config build.jobs=9 get {Z} build", None),
    (f"get {Z} -v --locked --color never build.jobs", None),
    (f"get {Z} --color always nothing.here", None),
    ("get --color always", None),
    ("get build.rustflags --show-origin", member),
    ("get --merged no", member),
    (f"get {Z}", not_toml),
    ("get", not_toml),
    ("", None),
    ("--help", None),
    ("get --help", None),
    ("get -h", None),
    (f"nope {Z}", None),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--blaze", default=os.path.join(ROOT, "target", "release", "ra-jit"))
    ap.add_argument("--out")
    opts = ap.parse_args()
    cases = []
    for args, first in CASES:
        outs = {}
        for who in ("cargo", "blaze"):
            work = tempfile.mkdtemp(prefix="own-config-")
            top = os.path.join(work, "ws")
            write(os.path.join(top, ".cargo", "config.toml"), TOP)
            write(os.path.join(top, "a", ".cargo", "config.toml"), MEMBER)
            env = {k: v for k, v in os.environ.items()
                   if not k.startswith("CARGO_") or k == "CARGO_HOME"}
            env.update({"CARGO_BUILD_JOBS": "4", "CARGO_ENV_X": "it's"})
            cwd = os.path.join(top, (first(top) if first else None) or "")
            argv = ["cargo"] + (["blaze"] if who == "blaze" else []) + ["config", *args.split()]
            if who == "blaze":
                env["PATH"] = os.path.dirname(os.path.abspath(opts.blaze)) + os.pathsep + env["PATH"]
            done = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, env=env)
            outs[who] = (done.returncode, done.stdout.replace(work, "@"), done.stderr.replace(work, "@"))
            shutil.rmtree(work)
        a, b = outs["cargo"], outs["blaze"]
        why = None
        for at, what in enumerate(("exit status", "stdout", "stderr")):
            if a[at] != b[at]:
                why = f"{what}: cargo {str(a[at])[:300]!r}, blaze {str(b[at])[:300]!r}"
                break
        name = f"config {args}" + (f" ({first.__name__})" if first else "")
        verdict = "equal" if why is None else "different"
        cases.append({"case": name, "verdict": verdict, "why": why})
        print(f"{verdict:10} {name}" + (f"  ({why})" if why else ""), flush=True)
    equal = sum(1 for c in cases if c["verdict"] == "equal")
    print(f"\n{equal}/{len(cases)} equal")
    if opts.out:
        with open(opts.out, "w") as f:
            json.dump(cases, f, indent=1)
    return 0 if equal == len(cases) else 1


if __name__ == "__main__":
    sys.exit(main())
