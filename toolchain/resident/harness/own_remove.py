#!/usr/bin/env python3
"""own_remove.py: `cargo blaze remove` against `cargo remove`. cargo is the oracle; blaze never
runs it outside this harness.

usage: own_remove.py --ripgrep DIR [--blaze BIN] [--out FILE]

Two workspaces, each copied afresh for every case and every side: ripgrep (DIR, a checkout
with its `Cargo.lock`, whose packages are fetched), with registry, path and per-target
dependencies, and a made-up one of path packages whose root manifest has
`[workspace.dependencies]`, `[profile.*.package]` and `[patch]` entries and whose member's
features name its dependencies every way a feature can (`dep:`, `?/`, `/`), with comments
inside the arrays. Each case runs `cargo remove ARGS` and `BIN cargo remove ARGS` (BIN:
`ra-jit`, default `target/release/ra-jit`) offline, comparing the exit status, standard
output and standard error, and then every `Cargo.toml` and `Cargo.lock` of the copy.
"""
import argparse, json, os, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

MADE = {
    "Cargo.toml": """\
[workspace]
members = ["a", "b"]
resolver = "2"

[workspace.dependencies]
c = { path = "c" } # the c
d = { path = "d" }
unused = { path = "unused" }

[profile.dev.package.c]
opt-level = 1

[profile.dev.package."d@0.1.0"]
opt-level = 2

[profile.dev.package."*"]
opt-level = 3

[patch.crates-io]
e = { path = "e" }
""",
    "a/Cargo.toml": """\
[package]
name = "a"
version = "0.1.0"
edition = "2021"

[dependencies]
c = { workspace = true, optional = true }
d.workspace = true
f = { path = "../f" }
h = { path = "../h", optional = true }

[dev-dependencies]
d = { workspace = true }

[target.'cfg(unix)'.dependencies]
g = { path = "../g" }

[features]
default = ["useh", "d/x"]
useh = ["h", "h/x"]
withc = ["dep:c", "c?/x"]
x = [
    # keeps this line
    "d/x", # and this one
    "c/x",
    "h?/x",
]
""",
    "b/Cargo.toml": """\
[package]
name = "b"
version = "0.1.0"
edition = "2021"

[dependencies]
d = { workspace = true }
""",
}
for name in ("c", "d", "e", "f", "g", "h", "unused"):
    MADE[f"{name}/Cargo.toml"] = (f"[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n"
                                   "[features]\nx = []\n")
for path in list(MADE):
    MADE[path.replace("Cargo.toml", "src/lib.rs")] = "\n"

JEMALLOC = "cfg(all(target_env = \"musl\", target_pointer_width = \"64\"))"

CASES = [
    ("ripgrep", ".", ["textwrap"]),
    ("ripgrep", ".", ["serde_json"]),
    ("ripgrep", ".", ["textwrap", "log"]),
    ("ripgrep", ".", ["grep"]),
    ("ripgrep", ".", ["--dev", "walkdir"]),
    ("ripgrep", ".", ["--dev", "serde", "serde_derive"]),
    ("ripgrep", ".", ["tikv-jemallocator"]),
    ("ripgrep", ".", ["--target", JEMALLOC, "tikv-jemallocator"]),
    ("ripgrep", ".", ["--build", "walkdir"]),
    ("ripgrep", ".", ["-n", "textwrap"]),
    ("ripgrep", ".", ["--dry-run", "nope"]),
    ("ripgrep", ".", ["--locked", "textwrap"]),
    ("ripgrep", ".", ["--locked", "log"]),
    ("ripgrep", ".", ["nope"]),
    ("ripgrep", ".", ["--dev", "nope2"]),
    ("ripgrep", ".", ["-q", "textwrap"]),
    ("ripgrep", ".", ["rm"]),
    ("ripgrep", ".", ["-p", "grep-cli", "log"]),
    ("ripgrep", ".", ["-p", "nope", "log"]),
    ("ripgrep", "crates/searcher", ["memmap2"]),
    ("ripgrep", ".", ["--manifest-path", "crates/searcher/Cargo.toml", "encoding_rs"]),
    ("ripgrep", ".", []),
    ("ripgrep", ".", ["--dev", "--build", "x"]),
    ("ripgrep", ".", ["--build", "-q", "--dev", "x"]),
    ("ripgrep", ".", ["--target", "foo", "-v", "-v", "--config", "a.b=1"]),
    ("ripgrep", ".", ["--dry-run", "-p"]),
    ("ripgrep", ".", ["textwrap", "-p"]),
    ("ripgrep", ".", ["-p", "--dev", "walkdir"]),
    ("made", ".", ["-p", "--", "c"]),
    ("ripgrep", ".", ["--help"]),
    ("made", "a", ["c"]),
    ("made", "a", ["d"]),
    ("made", "a", ["--dev", "d"]),
    ("made", "a", ["d", "--dev"]),
    ("made", "a", ["f"]),
    ("made", "a", ["h"]),
    ("made", "a", ["g"]),
    ("made", "a", ["--target", "cfg(unix)", "g"]),
    ("made", "a", ["c", "d", "f"]),
    ("made", "b", ["d"]),
    ("made", ".", ["d"]),
    ("made", ".", ["-p", "a", "c"]),
    ("made", ".", ["-p", "b", "d"]),
]


def files(top):
    out = {}
    for dirpath, dirnames, filenames in os.walk(top):
        dirnames[:] = [d for d in dirnames if d not in ("target", ".git")]
        for name in filenames:
            if name in ("Cargo.toml", "Cargo.lock"):
                path = os.path.join(dirpath, name)
                with open(path) as f:
                    out[os.path.relpath(path, top)] = f.read()
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ripgrep", required=True)
    ap.add_argument("--blaze", default=os.path.join(ROOT, "target", "release", "ra-jit"))
    ap.add_argument("--out")
    opts = ap.parse_args()
    env = {k: v for k, v in os.environ.items() if not k.startswith("CARGO_") or k == "CARGO_HOME"}
    templates = tempfile.mkdtemp(prefix="own-remove-")
    shutil.copytree(opts.ripgrep, os.path.join(templates, "ripgrep"),
                    ignore=shutil.ignore_patterns("target", ".git"))
    made = os.path.join(templates, "made")
    for path, text in MADE.items():
        os.makedirs(os.path.dirname(os.path.join(made, path)), exist_ok=True)
        with open(os.path.join(made, path), "w") as f:
            f.write(text)
    done = subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=made, env=env,
                          capture_output=True, text=True)
    if done.returncode != 0:
        sys.exit(f"the made-up workspace's lock file: {done.stderr}")
    cases = []
    for fixture, at, args in CASES:
        outs = {}
        for who in ("cargo", "blaze"):
            work = tempfile.mkdtemp(prefix="own-remove-case-")
            top = os.path.join(work, "ws")
            shutil.copytree(os.path.join(templates, fixture), top)
            argv = (["cargo"] if who == "cargo" else [opts.blaze, "cargo"]) + ["remove", "--offline", *args]
            done = subprocess.run(argv, cwd=os.path.join(top, at), capture_output=True, text=True, env=env)
            outs[who] = (done.returncode, done.stdout, done.stderr.replace(work, "@"), files(top))
            shutil.rmtree(work)
        a, b = outs["cargo"], outs["blaze"]
        why = None
        for n, what in enumerate(("exit status", "stdout", "stderr")):
            if a[n] != b[n]:
                why = f"{what}: cargo {str(a[n])[:400]!r}, blaze {str(b[n])[:400]!r}"
                break
        if why is None:
            for path in sorted(set(a[3]) | set(b[3])):
                if a[3].get(path) != b[3].get(path):
                    why = f"{path}: cargo {str(a[3].get(path))[:600]!r}, blaze {str(b[3].get(path))[:600]!r}"
                    break
        name = f"{fixture}/{at}: remove {' '.join(args)}"
        verdict = "equal" if why is None else "different"
        cases.append({"case": name, "verdict": verdict, "why": why})
        print(f"{verdict:10} {name}" + (f"  ({why})" if why else ""), flush=True)
    shutil.rmtree(templates)
    equal = sum(1 for c in cases if c["verdict"] == "equal")
    print(f"\n{equal}/{len(cases)} equal")
    if opts.out:
        with open(opts.out, "w") as f:
            json.dump(cases, f, indent=1)
    return 0 if equal == len(cases) else 1


if __name__ == "__main__":
    sys.exit(main())
