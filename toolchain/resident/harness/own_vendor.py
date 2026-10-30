#!/usr/bin/env python3
"""own_vendor.py: `cargo blaze vendor` against `cargo vendor`. cargo is the oracle; blaze never
runs it outside this harness.

usage: own_vendor.py --fd DIR --ripgrep DIR [--blaze BIN] [--out FILE]

fd and ripgrep (checkouts with their `Cargo.lock`, whose packages are fetched) and a made-up
workspace of path packages are copied afresh for every case and every side, side by side,
and each case runs `cargo vendor ARGS` and `BIN cargo vendor ARGS` (BIN: `ra-jit`, default
`target/release/ra-jit`) offline in one of them, comparing the exit status, standard output
and standard error, and then every file under the copies: its path and sha256, the vendored
packages, their `.cargo-checksum.json` and the lock files. A case may first change the copy:
a vendor directory with a stale package and a dot file in it, or one vendored already.
"""
import argparse, hashlib, json, os, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

MADE = {
    "Cargo.toml": "[workspace]\nmembers = [\"a\"]\nresolver = \"2\"\n",
    "a/Cargo.toml": "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nb = { path = \"../b\" }\n",
    "a/src/lib.rs": "\n",
    "b/Cargo.toml": "[package]\nname = \"b\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    "b/src/lib.rs": "\n",
}


def stale(top):
    os.makedirs(os.path.join(top, "vendor", "old-package", "src"))
    with open(os.path.join(top, "vendor", "old-package", "src", "lib.rs"), "w") as f:
        f.write("\n")
    with open(os.path.join(top, "vendor", ".keep"), "w") as f:
        f.write("kept\n")


CASES = [
    ("fd", [], None),
    ("fd", ["--versioned-dirs"], None),
    ("fd", ["third_party/crates"], None),
    ("fd", ["-q"], None),
    ("fd", ["--locked"], None),
    ("fd", [], stale),
    ("fd", ["--no-delete"], stale),
    ("fd", ["again"], None),
    ("fd", ["--manifest-path", "Cargo.toml", "v"], None),
    ("ripgrep", [], None),
    ("ripgrep", ["--versioned-dirs"], None),
    ("ripgrep", ["-s", "../fd/Cargo.toml"], None),
    ("made", [], None),
    ("made", ["-q"], None),
    ("fd", ["-s", "nope/Cargo.toml"], None),
    ("fd", ["--relative-path"], None),
    ("fd", ["a", "b"], None),
    ("fd", ["--help"], None),
]


def snapshot(top):
    out = {}
    for dirpath, dirnames, filenames in os.walk(top):
        dirnames[:] = [d for d in dirnames if d not in ("target", ".git")]
        rel = os.path.relpath(dirpath, top)
        if not filenames and not dirnames:
            out[rel + "/"] = "empty directory"
        for name in filenames:
            path = os.path.join(dirpath, name)
            with open(path, "rb") as f:
                out[os.path.relpath(path, top)] = hashlib.sha256(f.read()).hexdigest()
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--fd", required=True)
    ap.add_argument("--ripgrep", required=True)
    ap.add_argument("--blaze", default=os.path.join(ROOT, "target", "release", "ra-jit"))
    ap.add_argument("--out")
    opts = ap.parse_args()
    env = {k: v for k, v in os.environ.items() if not k.startswith("CARGO_") or k == "CARGO_HOME"}
    templates = tempfile.mkdtemp(prefix="own-vendor-")
    for name, src in (("fd", opts.fd), ("ripgrep", opts.ripgrep)):
        shutil.copytree(src, os.path.join(templates, name), ignore=shutil.ignore_patterns("target", ".git", "vendor"))
    for path, text in MADE.items():
        os.makedirs(os.path.dirname(os.path.join(templates, "made", path)), exist_ok=True)
        with open(os.path.join(templates, "made", path), "w") as f:
            f.write(text)
    done = subprocess.run(["cargo", "generate-lockfile", "--offline"], cwd=os.path.join(templates, "made"),
                          env=env, capture_output=True, text=True)
    if done.returncode != 0:
        sys.exit(f"the made-up workspace's lock file: {done.stderr}")
    cases = []
    for fixture, args, first in CASES:
        outs = {}
        for who in ("cargo", "blaze"):
            work = tempfile.mkdtemp(prefix="own-vendor-case-")
            for name in ("fd", "ripgrep", "made"):
                shutil.copytree(os.path.join(templates, name), os.path.join(work, name))
            top = os.path.join(work, fixture)
            if first:
                first(top)
            argv = (["cargo"] if who == "cargo" else [opts.blaze, "cargo"]) + ["vendor", "--offline"]
            runs = 2 if args == ["again"] else 1
            for _ in range(runs):
                done = subprocess.run(argv + [a for a in args if a != "again"], cwd=top,
                                      capture_output=True, text=True, env=env)
            outs[who] = (done.returncode, done.stdout, done.stderr.replace(work, "@"), snapshot(work))
            shutil.rmtree(work)
        a, b = outs["cargo"], outs["blaze"]
        why = None
        for n, what in enumerate(("exit status", "stdout", "stderr")):
            if a[n] != b[n]:
                why = f"{what}: cargo {str(a[n])[:500]!r}, blaze {str(b[n])[:500]!r}"
                break
        if why is None:
            paths = sorted(set(a[3]) | set(b[3]))
            differ = [p for p in paths if a[3].get(p) != b[3].get(p)]
            if differ:
                why = f"{len(differ)} files differ, first {differ[0]}: cargo {a[3].get(differ[0])}, blaze {b[3].get(differ[0])}"
        name = f"{fixture}: vendor {' '.join(args)}" + (f" ({first.__name__})" if first else "")
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
