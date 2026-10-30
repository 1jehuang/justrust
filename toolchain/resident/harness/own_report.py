#!/usr/bin/env python3
"""own_report.py: `cargo blaze report` against `cargo report`. cargo is the oracle; blaze never
runs it outside this harness.

usage: own_report.py [--blaze BIN] [--out FILE]

A made-up package is given the file builds leave their future-incompatibility reports in,
written here as cargo writes it (two reports, colored, one with a terminal link), and each
case runs `cargo report ARGS` and `BIN cargo report ARGS` (BIN: `ra-jit`, default
`target/release/ra-jit`) in it, comparing the exit status, standard output and standard
error. A case may first change the package: no reports, a file of a later cargo, one that is
not JSON, or a `build.build-dir` the reports are in.
"""
import argparse, json, os, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

WARNING = ("\x1b[1m\x1b[33mwarning\x1b[0m\x1b[1m: this was previously accepted\x1b[0m\n"
           "  \x1b[1m\x1b[94m-->\x1b[0m src/lib.rs:1:1\n"
           "   = note: see \x1b]8;;https://github.com/rust-lang/rust/issues/1\x1b\\issue #1\x1b]8;;\x1b\\\n")
REPORTS = {
    "version": 0,
    "next_id": 3,
    "reports": [
        {"id": 1, "suggestion_message": "\nTo solve this problem, you can try the following approaches:\n\n",
         "per_package": {"old@0.1.0": "The package `old v0.1.0` currently triggers the following future incompatibility lints:\n"
                         + "".join(f"> {l}\n" for l in WARNING.splitlines())}},
        {"id": 2, "suggestion_message": "",
         "per_package": {"a@1.0.0": "The package `a v1.0.0` currently triggers the following future incompatibility lints:\n> one\n",
                         "b@2.0.0": "The package `b v2.0.0` currently triggers the following future incompatibility lints:\n> two\n"}},
    ],
}


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


def no_reports(pkg):
    os.remove(os.path.join(pkg, "target", ".future-incompat-report.json"))


def later_cargo(pkg):
    write(os.path.join(pkg, "target", ".future-incompat-report.json"), json.dumps({**REPORTS, "version": 1}))


def not_json(pkg):
    write(os.path.join(pkg, "target", ".future-incompat-report.json"), "{")


def target_dir_variable(pkg):
    shutil.move(os.path.join(pkg, "target"), os.path.join(pkg, "by-variable"))
    return {"CARGO_TARGET_DIR": os.path.join(pkg, "by-variable")}


def build_dir(pkg):
    shutil.move(os.path.join(pkg, "target"), os.path.join(pkg, "elsewhere"))
    write(os.path.join(pkg, ".cargo", "config.toml"), "[build]\nbuild-dir = \"elsewhere\"\n")


CASES = [
    ("future-incompatibilities", None),
    ("future-incompat", None),
    ("future-incompat --id 1", None),
    ("future-incompat --id=2", None),
    ("future-incompat --id 9", None),
    ("future-incompat --id x", None),
    ("future-incompat --id 2 -p b@2.0.0", None),
    ("future-incompat --id 2 --package a@1.0.0", None),
    ("future-incompat -p nope", None),
    ("future-incompat --id 1 --color always", None),
    ("future-incompat --id 1 --color never", None),
    ("future-incompat", no_reports),
    ("future-incompat", later_cargo),
    ("future-incompat", not_json),
    ("future-incompat", build_dir),
    ("future-incompat", target_dir_variable),
    ("future-incompat --id x", no_reports),
    ("future-incompat --color always", no_reports),
    ("timings", None),
    ("sessions", None),
    ("rebuilds", None),
    ("", None),
    ("nothing-such", None),
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
            work = tempfile.mkdtemp(prefix="own-report-")
            pkg = os.path.join(work, "pkg")
            write(os.path.join(pkg, "Cargo.toml"), "[package]\nname = \"pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n")
            write(os.path.join(pkg, "src", "lib.rs"), "\n")
            write(os.path.join(pkg, "target", ".future-incompat-report.json"), json.dumps(REPORTS))
            env = {k: v for k, v in os.environ.items() if k != "CARGO_TARGET_DIR"}
            if first:
                env.update(first(pkg) or {})
            argv = (["cargo"] if who == "cargo" else [opts.blaze, "cargo"]) + ["report", *args.split()]
            done = subprocess.run(argv, cwd=pkg, capture_output=True, text=True, env=env)
            outs[who] = (done.returncode, done.stdout, done.stderr.replace(work, "@"))
            shutil.rmtree(work)
        a, b = outs["cargo"], outs["blaze"]
        why = None
        for at, what in enumerate(("exit status", "stdout", "stderr")):
            if a[at] != b[at]:
                why = f"{what}: cargo {str(a[at])[:300]!r}, blaze {str(b[at])[:300]!r}"
                break
        name = f"report {args}" + (f" ({first.__name__})" if first else "")
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
