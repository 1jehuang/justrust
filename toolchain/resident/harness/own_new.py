#!/usr/bin/env python3
"""own_new.py: `cargo blaze new` and `cargo blaze init` against cargo's. cargo is the oracle;
blaze never runs it outside this harness.

usage: own_new.py [--blaze BIN] [--out FILE]

Each case of CASES makes the same directory twice, runs `cargo ARGS` in one and `BIN cargo
ARGS` (BIN: `ra-jit`, default `target/release/ra-jit`) in the other, with a home and a cargo
home of their own, and compares the exit status, what each prints, and every file each leaves
(a git repository by its `HEAD` and whether it is one). A case is equal or different; the
first difference of each that is not is printed, `--out` writes them all as JSON, and the exit
status is 1 when any case is not equal.
"""
import argparse, json, os, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

WORKSPACE = """[workspace]
resolver = "3"
members = ["crates/a"]

[workspace.package]
version = "1.2.3"
edition = "2021"
license = "MIT"

[workspace.lints.rust]
unsafe_code = "forbid"
"""

A = '[package]\nname = "a"\nversion.workspace = true\nedition.workspace = true\n'

# (name, the files made first, the directory to run in, the arguments).
CASES = [
    ("bin", {}, ".", "new foo"),
    ("lib", {}, ".", "new --lib foo"),
    ("edition and name", {}, ".", "new --edition 2021 --name bar foo"),
    ("no vcs", {}, ".", "new --vcs none foo"),
    ("registry", {}, ".", "new --registry my-registry foo"),
    ("deep path", {}, ".", "new one/two/foo"),
    ("exists", {"foo/x": ""}, ".", "new foo"),
    ("bin and lib", {}, ".", "new --bin --lib foo"),
    ("bad edition", {}, ".", "new --edition 2019 foo"),
    ("bad vcs", {}, ".", "new --vcs svn foo"),
    ("near vcs", {}, ".", "new --vcs gti foo"),
    ("near edition", {}, ".", "new --edition 2023 foo"),
    ("init bad edition", {}, ".", "init --edition 15"),
    ("no path", {}, ".", "new"),
    ("keyword", {}, ".", "new fn"),
    ("test", {}, ".", "new test"),
    ("std", {}, ".", "new std"),
    ("build dir name, bin", {}, ".", "new deps"),
    ("build dir name, lib", {}, ".", "new --lib deps"),
    ("windows name", {}, ".", "new --lib con"),
    ("upper case", {}, ".", "new Foo"),
    ("digit first", {}, ".", "new 1foo"),
    ("non-ascii", {}, ".", "new café"),
    ("bad char", {}, ".", "new foo.bar"),
    ("quiet", {}, ".", "new -q foo"),
    ("in a git repo", {"repo/.git/HEAD": "ref: refs/heads/main\n"}, "repo", "new foo"),
    ("init empty", {"foo/.keep": ""}, "foo", "init"),
    ("init path", {"foo/.keep": ""}, ".", "init foo"),
    ("init lib.rs", {"foo/src/lib.rs": "pub fn f() {}\n"}, "foo", "init"),
    ("init main.rs", {"foo/main.rs": "fn main() {}\n"}, "foo", "init"),
    ("init named.rs bin", {"foo/foo.rs": "fn main() {}\n"}, "foo", "init"),
    ("init named.rs lib", {"foo/src/foo.rs": "pub fn f() {}\n"}, "foo", "init"),
    ("init both", {"foo/src/main.rs": "fn main() {}\n", "foo/src/lib.rs": ""}, "foo", "init"),
    ("init both as lib", {"foo/src/main.rs": "fn main() {}\n", "foo/src/lib.rs": ""}, "foo",
     "init --lib"),
    ("init lib asked of bin", {"foo/src/main.rs": "fn main() {}\n"}, "foo", "init --lib"),
    ("init two bins", {"foo/src/main.rs": "fn main() {}\n", "foo/main.rs": "fn main() {}\n"},
     "foo", "init"),
    ("init existing package", {"foo/Cargo.toml": A}, "foo", "init"),
    ("init gitignore", {"foo/.gitignore": "/target\n*.swp\n"}, "foo", "init"),
    ("init gitignore no newline", {"foo/.gitignore": "*.swp"}, "foo", "init"),
    ("init hgignore", {"foo/.hg/x": ""}, "foo", "init --vcs none"),
    ("init two vcs", {"foo/.git/x": "", "foo/.hg/x": ""}, "foo", "init"),
    ("workspace member", {"ws/Cargo.toml": WORKSPACE, "ws/crates/a/Cargo.toml": A,
                          "ws/crates/a/src/lib.rs": ""}, "ws", "new crates/b"),
    ("workspace member, edition", {"ws/Cargo.toml": WORKSPACE, "ws/crates/a/Cargo.toml": A,
                                   "ws/crates/a/src/lib.rs": ""}, "ws",
     "new --edition 2024 --lib crates/b"),
    ("workspace glob", {"ws/Cargo.toml": WORKSPACE.replace('"crates/a"', '"crates/*"'),
                        "ws/crates/a/Cargo.toml": A, "ws/crates/a/src/lib.rs": ""}, "ws",
     "new crates/b"),
    ("workspace unsorted", {"ws/Cargo.toml": WORKSPACE.replace('"crates/a"', '"z", "crates/a"'),
                            "ws/crates/a/Cargo.toml": A, "ws/crates/a/src/lib.rs": "",
                            "ws/z/Cargo.toml": A.replace('"a"', '"z"'), "ws/z/src/lib.rs": ""},
     "ws", "new crates/b"),
    ("workspace exclude", {"ws/Cargo.toml": WORKSPACE.replace('members = ["crates/a"]',
                                                               'members = ["crates/a"]\nexclude = ["other"]'),
                           "ws/crates/a/Cargo.toml": A, "ws/crates/a/src/lib.rs": ""}, "ws",
     "new other"),
    ("workspace no members", {"ws/Cargo.toml": "[workspace]\n"}, "ws", "new b"),
    ("workspace broken member", {"ws/Cargo.toml": '[workspace]\nmembers = ["gone"]\n'}, "ws",
     "new b"),
    ("in a package", {"p/Cargo.toml": '[package]\nname = "p"\nversion = "0.1.0"\n',
                      "p/src/lib.rs": ""}, "p", "new inner"),
    ("init in a workspace", {"ws/Cargo.toml": WORKSPACE, "ws/crates/a/Cargo.toml": A,
                             "ws/crates/a/src/lib.rs": "", "ws/crates/b/.keep": ""}, "ws/crates/b",
     "init"),
]
def made(root, files):
    os.makedirs(root)
    for path, text in files.items():
        full = os.path.join(root, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w") as f:
            f.write(text)


def left(root):
    """Every file under `root`, by its path: its text, and a git repository by its `HEAD`."""
    found = {}
    for dir, dirs, files in os.walk(root):
        rel = os.path.relpath(dir, root)
        if ".git" in dirs and os.path.exists(os.path.join(dir, ".git", "HEAD")):
            dirs.remove(".git")
            with open(os.path.join(dir, ".git", "HEAD")) as f:
                found[os.path.join(rel, ".git")] = f"repository, HEAD {f.read().strip()}"
        for name in files:
            with open(os.path.join(dir, name), "rb") as f:
                found[os.path.normpath(os.path.join(rel, name))] = f.read().decode(errors="replace")
    return found


def run(argv, cwd, home):
    env = {**os.environ, "HOME": home, "CARGO_HOME": os.path.join(home, ".cargo"),
           "RUSTUP_HOME": os.environ.get("RUSTUP_HOME", os.path.expanduser("~/.rustup")),
           "CARGO_TERM_COLOR": "never"}
    for name in ("GIT_DIR", "GIT_WORK_TREE", "CARGO_TARGET_DIR"):
        env.pop(name, None)
    done = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, env=env)
    return done.returncode, done.stdout, done.stderr


def first_difference(what, a, b):
    a_lines, b_lines = a.splitlines(), b.splitlines()
    for at, (x, y) in enumerate(zip(a_lines, b_lines)):
        if x != y:
            return f"{what} line {at + 1}: cargo `{x[:200]}`, blaze `{y[:200]}`"
    if len(a_lines) != len(b_lines):
        return f"{what}: cargo has {len(a_lines)} lines, blaze {len(b_lines)}"
    return None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--blaze", default=os.path.join(ROOT, "target", "release", "ra-jit"))
    ap.add_argument("--out")
    opts = ap.parse_args()
    results = []
    with tempfile.TemporaryDirectory() as tmp:
        for at, (name, files, where, args) in enumerate(CASES):
            seen = []
            for side, argv in (("cargo", ["cargo"]), ("blaze", [opts.blaze, "cargo"])):
                root = os.path.join(tmp, str(at), side, "case")
                home = os.path.join(tmp, str(at), side, "home")
                made(root, files)
                os.makedirs(os.path.join(home, ".cargo"))
                status, out, err = run(argv + args.split(), os.path.join(root, where), home)
                seen.append((status, out.replace(root, "@"), err.replace(root, "@"), left(root)))
            cargo, blaze = seen
            why = None
            if cargo[0] != blaze[0]:
                why = f"cargo exits {cargo[0]}, blaze {blaze[0]}: {(blaze[2] or cargo[2]).strip()[:300]}"
            why = why or first_difference("stdout", cargo[1], blaze[1])
            why = why or first_difference("stderr", cargo[2], blaze[2])
            if not why and cargo[3] != blaze[3]:
                for path in sorted(set(cargo[3]) | set(blaze[3])):
                    a, b = cargo[3].get(path), blaze[3].get(path)
                    if a != b:
                        why = (f"{path}: cargo leaves {a!r:.200}, blaze {b!r:.200}")
                        break
            results.append({"case": name, "args": args, "equal": why is None, "why": why})
            if why:
                print(f"DIFF {name}: {args}\n  {why}")
    equal = sum(r["equal"] for r in results)
    print(f"{equal}/{len(results)} cases equal")
    if opts.out:
        with open(opts.out, "w") as f:
            json.dump(results, f, indent=1)
    sys.exit(0 if equal == len(results) else 1)


if __name__ == "__main__":
    main()
