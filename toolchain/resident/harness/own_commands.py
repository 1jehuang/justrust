#!/usr/bin/env python3
"""own_commands.py: the cargo subcommands blaze answers itself that are not builds, against
cargo's, over workspaces. cargo is the oracle; blaze never runs it outside this harness.

usage: own_commands.py [--blaze BIN] [--out FILE] DIR...

Each DIR is a workspace with its Cargo.lock, whose packages are fetched already. In each, every
case of CASES is run as `cargo ARGS` and as `BIN cargo ARGS` (BIN: `ra-jit`, default
`target/release/ra-jit`), and standard output (as JSON for the commands that print JSON),
standard error and the exit status compared. The `clean` cases run in two copies of a made-up
target directory, one for each, and compare what each says it removed and what is left.
`cargo tree` has harness/own_tree.py.

A case is equal or different; the first difference of each that is not is printed, `--out`
writes them all as JSON, and the exit status is 1 when any case is not equal.
"""
import argparse, json, os, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# (arguments, whether stdout is JSON). `fetch` is held to its status and its standard output
# only: cargo 1.101 prints its manifest lints (`cargo::redundant_homepage`) for it, which blaze
# has not written.
CASES = [
    ("metadata --format-version 1", True),
    ("metadata --format-version 1 --no-deps", True),
    ("metadata --format-version 1 --filter-platform x86_64-pc-windows-msvc", True),
    ("metadata --format-version 1 --all-features", True),
    ("locate-project", True),
    ("locate-project --workspace", True),
    ("locate-project --message-format plain", False),
    ("verify-project", True),
    ("pkgid", False),
    ("pkgid serde", False),
    ("pkgid -p libc", False),
    ("pkgid serde@1", False),
    ("pkgid no-such-package", False),
    ("pkgid https://github.com/rust-lang/crates.io-index#serde", False),
    ("read-manifest", True),
    ("read-manifest --manifest-path Cargo.toml", True),
    ("fetch", False),
    ("fetch --target x86_64-pc-windows-msvc", False),
]

CLEANS = [
    "clean",
    "clean --release",
    "clean --profile dev",
    "clean --profile bench",
    "clean --profile no-such-profile",
    "clean --doc",
    "clean --dry-run",
    "clean --doc --target x86_64-unknown-linux-gnu",
]


def run(argv, cwd):
    # cargo would write `.rustc_info.json`, what rustc said, into the target directory it cleans.
    env = {**os.environ, "CARGO_CACHE_RUSTC_INFO": "0"}
    done = subprocess.run(argv, cwd=cwd, capture_output=True, text=True, env=env)
    return done.returncode, done.stdout, done.stderr


def normal(out, as_json):
    if not as_json:
        return out
    try:
        return json.dumps(json.loads(out), sort_keys=True, indent=0)
    except ValueError:
        return out


def first_difference(what, a, b):
    a_lines, b_lines = a.splitlines(), b.splitlines()
    for at, (x, y) in enumerate(zip(a_lines, b_lines)):
        if x != y:
            return f"{what} line {at + 1}: cargo `{x[:200]}`, blaze `{y[:200]}`"
    if len(a_lines) != len(b_lines):
        return f"{what}: cargo prints {len(a_lines)} lines, blaze {len(b_lines)}"
    return None


def compare(cargo, blaze, as_json, root_a="", root_b="", with_stderr=True):
    if cargo[0] != blaze[0]:
        return f"cargo exits {cargo[0]}, blaze {blaze[0]}: {(blaze[2] or cargo[2]).strip()[:300]}"
    out = first_difference("stdout", normal(cargo[1], as_json), normal(blaze[1], as_json))
    if not with_stderr:
        return out
    if root_a:
        cargo, blaze = cargo[:2] + (cargo[2].replace(root_a, "@"),), blaze[:2] + (blaze[2].replace(root_b, "@"),)
    return out or first_difference("stderr", cargo[2], blaze[2])


def fake_target(dir):
    """A target directory as builds leave one: profiles, a target triple's, docs, hard links."""
    with open(os.path.join(dir, "CACHEDIR.TAG"), "w") as f:
        f.write("Signature: 8a477f597d28d172789f06886806bc55\n")
    for sub in ["debug/deps", "debug/build/x-1", "release/deps", "doc/x",
                "x86_64-unknown-linux-gnu/doc/y", "x86_64-unknown-linux-gnu/debug", "empty/a/b"]:
        os.makedirs(os.path.join(dir, sub), exist_ok=True)
    for at, rel in enumerate(["debug/deps/libx.rlib", "debug/build/x-1/out", "release/deps/x",
                              "doc/x/index.html", "x86_64-unknown-linux-gnu/doc/y/a.html",
                              "x86_64-unknown-linux-gnu/debug/z", "debug/.fingerprint"]):
        with open(os.path.join(dir, rel), "wb") as f:
            f.write(b"x" * (1000 * (at + 1) ** 3))
    os.link(os.path.join(dir, "debug/deps/libx.rlib"), os.path.join(dir, "debug/libx.rlib"))
    os.symlink("deps/libx.rlib", os.path.join(dir, "debug/link"))


def left(dir):
    if not os.path.exists(dir):
        return []
    return sorted(os.path.relpath(os.path.join(d, n), dir)
                  for d, ds, fs in os.walk(dir) for n in ds + fs)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("dirs", nargs="+")
    ap.add_argument("--blaze", default=os.path.join(ROOT, "target", "release", "ra-jit"))
    ap.add_argument("--out")
    opts = ap.parse_args()
    cases = []

    def record(name, why):
        verdict = "equal" if why is None else "different"
        cases.append({"case": name, "verdict": verdict, "why": why})
        print(f"{verdict:10} {name}" + (f"  ({why})" if why else ""), flush=True)

    for d in opts.dirs:
        app = os.path.basename(os.path.abspath(d))
        for args, as_json in CASES:
            cargo = run(["cargo", *args.split()], d)
            blaze = run([opts.blaze, "cargo", *args.split()], d)
            with_stderr = not args.startswith("fetch")
            record(f"{app}: {args}", compare(cargo, blaze, as_json, with_stderr=with_stderr))
        for args in CLEANS:
            with tempfile.TemporaryDirectory() as tmp:
                a, b = os.path.join(tmp, "a"), os.path.join(tmp, "b")
                for it in (a, b):
                    os.makedirs(it)
                    fake_target(it)
                cargo = run(["cargo", *args.split(), "--target-dir", a], d)
                blaze = run([opts.blaze, "cargo", *args.split(), "--target-dir", b], d)
                why = compare(cargo, blaze, False, a, b)
                if why is None and left(a) != left(b):
                    why = f"cargo leaves {left(a)}, blaze {left(b)}"
                record(f"{app}: {args}", why)
    equal = sum(1 for c in cases if c["verdict"] == "equal")
    print(f"\n{equal}/{len(cases)} equal")
    if opts.out:
        with open(opts.out, "w") as f:
            json.dump(cases, f, indent=1)
    return 0 if equal == len(cases) else 1


if __name__ == "__main__":
    sys.exit(main())
