#!/usr/bin/env python3
"""own_uninstall.py: `cargo blaze uninstall` against `cargo uninstall`. cargo is the oracle; blaze
never runs it outside this harness.

usage: own_uninstall.py [--blaze BIN] [--out FILE]

Three made-up packages (one with two programs, one with one, one with an example only) are
installed by `cargo install --path` into an install root, once. Each case copies that root
twice, runs `cargo uninstall ARGS` in one and `BIN cargo uninstall ARGS` (BIN: `ra-jit`,
default `target/release/ra-jit`) in the other, and compares the exit status, standard output
and error (with the roots named alike), the files left and the bytes of `.crates.toml` and
`.crates2.json`. A case may first change the root, as an older cargo or a lost file would.
"""
import argparse, json, os, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

PACKAGES = {
    "two": ("[[bin]]\nname = \"one\"\npath = \"src/one.rs\"\n\n[[bin]]\nname = \"other\"\npath = \"src/other.rs\"\n",
            {"src/one.rs": "fn main() {}\n", "src/other.rs": "fn main() {}\n"}),
    "solo": ("", {"src/main.rs": "fn main() {}\n"}),
    "lib-only": ("", {"src/lib.rs": "\n"}),
}


def drop_v2(root):
    os.remove(os.path.join(root, ".crates2.json"))


def drop_bin(root):
    os.remove(os.path.join(root, "bin", "solo"))


def empty(root):
    shutil.rmtree(root)
    os.makedirs(root)


# (name, arguments, where it runs: "root" or a package's directory, what is done first,
# whether the root is named by CARGO_INSTALL_ROOT instead of --root)
CASES = [
    ("by name", "solo", None, None, False),
    ("both programs", "two", None, None, False),
    ("one program", "two --bin one", None, None, False),
    ("one program with -p", "-p two --bin other", None, None, False),
    ("two packages", "two solo", None, None, False),
    ("one of two missing", "solo nope", None, None, False),
    ("not installed", "nope", None, None, False),
    ("a program it did not install", "two --bin solo", None, None, False),
    ("--bin with two specs", "two solo --bin one", None, None, False),
    ("by version", "solo@0.1.0", None, None, False),
    ("by a version it is not", "solo@0.2.0", None, None, False),
    ("the package of the directory", "", "solo", None, False),
    ("the package of the directory, not installed", "", "lib-only", None, False),
    ("quiet", "-q two", None, None, False),
    ("from CARGO_INSTALL_ROOT", "solo", None, None, True),
    ("an older cargo's root", "two --bin one", None, drop_v2, False),
    ("a program lost", "solo", None, drop_bin, False),
    ("an empty root", "solo", None, empty, False),
]


def run(argv, cwd, env):
    done = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, env=env)
    return done.returncode, done.stdout, done.stderr


def state(root):
    files = {}
    for d, ds, fs in os.walk(root):
        for n in fs:
            path = os.path.join(d, n)
            rel = os.path.relpath(path, root)
            files[rel] = open(path, "rb").read() if n.startswith(".crates") else None
    return files


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--blaze", default=os.path.join(ROOT, "target", "release", "ra-jit"))
    ap.add_argument("--out")
    opts = ap.parse_args()
    work = tempfile.mkdtemp(prefix="own-uninstall-")
    base = os.path.join(work, "base")
    for name, (extra, files) in PACKAGES.items():
        pkg = os.path.join(work, name)
        os.makedirs(os.path.join(pkg, "src"))
        with open(os.path.join(pkg, "Cargo.toml"), "w") as f:
            f.write(f"[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n{extra}")
        for rel, text in files.items():
            with open(os.path.join(pkg, rel), "w") as f:
                f.write(text)
        if name != "lib-only":
            done = subprocess.run(["cargo", "install", "-q", "--path", pkg, "--root", base],
                                  capture_output=True, text=True)
            if done.returncode:
                sys.exit(f"cargo install {name}: {done.stderr}")
    cases = []
    for name, args, where, first, by_env in CASES:
        roots = {}
        outs = {}
        for who in ("cargo", "blaze"):
            root = os.path.join(work, f"{who}-root")
            shutil.rmtree(root, ignore_errors=True)
            shutil.copytree(base, root, symlinks=True)
            if first:
                first(root)
            roots[who] = root
            env = {**os.environ}
            argv = (["cargo"] if who == "cargo" else [opts.blaze, "cargo"]) + ["uninstall", *args.split()]
            if by_env:
                env["CARGO_INSTALL_ROOT"] = root
            else:
                argv += ["--root", root]
            cwd = os.path.join(work, where) if where else work
            code, out, err = run(argv, cwd, env)
            outs[who] = (code, out, err.replace(root, "@ROOT"), state(root))
        a, b = outs["cargo"], outs["blaze"]
        why = None
        if a[0] != b[0]:
            why = f"cargo exits {a[0]}, blaze {b[0]}: {(b[2] or a[2]).strip()[:300]}"
        elif a[1] != b[1]:
            why = f"stdout: cargo {a[1][:200]!r}, blaze {b[1][:200]!r}"
        elif a[2] != b[2]:
            why = f"stderr: cargo {a[2][:400]!r}, blaze {b[2][:400]!r}"
        elif a[3] != b[3]:
            diff = sorted(k for k in set(a[3]) | set(b[3]) if a[3].get(k, 0) != b[3].get(k, 0))
            why = f"what is left differs: {diff}: cargo {[a[3].get(k) for k in diff]}, blaze {[b[3].get(k) for k in diff]}"
        verdict = "equal" if why is None else "different"
        cases.append({"case": name, "args": args, "verdict": verdict, "why": why})
        print(f"{verdict:10} {name}: uninstall {args}" + (f"  ({why})" if why else ""), flush=True)
    shutil.rmtree(work)
    equal = sum(1 for c in cases if c["verdict"] == "equal")
    print(f"\n{equal}/{len(cases)} equal")
    if opts.out:
        with open(opts.out, "w") as f:
            json.dump(cases, f, indent=1)
    return 0 if equal == len(cases) else 1


if __name__ == "__main__":
    sys.exit(main())
