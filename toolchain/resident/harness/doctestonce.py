#!/usr/bin/env python3
"""The doctests of a workspace of three libraries, run by `cargo test --doc` and by
`cargo ra-jit test --doc`: ra-jit loads the workspace once for all of them, and they end as
cargo's do, one library after another, each example reading where it runs and the variables
as it does under rustdoc.

    RA_JIT_BIN=../target/release/ra-jit python3 doctestonce.py

The steps: every example passes; an example of the first two libraries fails, and cargo stops
after the first; the same with `--no-fail-fast`; one of the first fails, with `--no-fail-fast`. In each, the exit status, the results of
each library in turn, the `Doc-tests` lines and cargo's lines of what failed have to be
cargo's, every example has to be ra-jit's (on macOS std's writable statics leave them to
rustc, #255), and ra-jit has to have loaded the workspace once.
A difference is WRONG and fails the run."""
import os
import re
import subprocess
import sys
import tempfile

from buildmodel import doctest_report

HERE = os.path.dirname(os.path.abspath(__file__))
RA_JIT = os.path.abspath(os.environ.get("RA_JIT_BIN", os.path.join(HERE, "..", "target", "release", "ra-jit")))

# An exit status, not a panic: where a panic in an example is said is not this step's to check.
FAILS = '/// if std::env::var_os("DOCTEST_FAIL").is_some() {\n///     std::process::exit(3);\n/// }\n'
FAILS_FIRST = FAILS.replace('var_os("DOCTEST_FAIL")', 'var_os("DOCTEST_FAIL").or(std::env::var_os("DOCTEST_FAIL_FIRST"))')
# Each library's examples read the package they run in and its name, at run time and at
# compile time; gamma's use beta, a dev-dependency of gamma alone.
WORKSPACE = {
    "Cargo.toml": '[workspace]\nmembers = ["alpha", "beta", "gamma"]\nresolver = "2"\n',
    "alpha/Cargo.toml": '[package]\nname = "alpha"\nversion = "0.1.0"\nedition = "2021"\n',
    "alpha/src/lib.rs": "//! ```\n//! assert_eq!(alpha::double(2), 4);\n//! ```\n\n"
    "/// Twice `n`.\n///\n/// ```\n" + FAILS_FIRST + "/// assert_eq!(alpha::double(3), 6);\n/// ```\n"
    "pub fn double(n: u32) -> u32 {\n    n * 2\n}\n\n"
    "/// ```\n/// assert!(std::env::current_dir().unwrap().ends_with(\"alpha\"));\n"
    "/// assert_eq!(std::env::var(\"CARGO_PKG_NAME\").unwrap(), \"alpha\");\n/// ```\npub fn here() {}\n",
    "beta/Cargo.toml": '[package]\nname = "beta"\nversion = "0.1.0"\nedition = "2021"\n\n'
    '[dependencies]\nalpha = { path = "../alpha" }\n',
    "beta/src/lib.rs": "/// ```\n" + FAILS + "/// assert_eq!(beta::quadruple(1), 4);\n"
    "/// assert!(std::env::current_dir().unwrap().ends_with(\"beta\"));\n"
    "/// assert_eq!(std::env::var(\"CARGO_PKG_NAME\").unwrap(), \"beta\");\n/// ```\n"
    "pub fn quadruple(n: u32) -> u32 {\n    alpha::double(alpha::double(n))\n}\n\n"
    "/// ```\n/// assert_eq!(env!(\"CARGO_PKG_NAME\"), \"beta\");\n/// ```\npub fn name() {}\n",
    "gamma/Cargo.toml": '[package]\nname = "gamma"\nversion = "0.1.0"\nedition = "2021"\n\n'
    '[dev-dependencies]\nbeta = { path = "../beta" }\n',
    "gamma/src/lib.rs": "/// ```\n/// assert_eq!(beta::quadruple(gamma::one()), 4);\n/// ```\n"
    "pub fn one() -> u32 {\n    1\n}\n",
}

STEPS = [
    ("every example passes", [], {}),
    ("examples fail", [], {"DOCTEST_FAIL": "1"}),
    ("examples fail, --no-fail-fast", ["--no-fail-fast"], {"DOCTEST_FAIL": "1"}),
    ("one fails, --no-fail-fast", ["--no-fail-fast"], {"DOCTEST_FAIL_FIRST": "1"}),
]


def tested(ws, command, args, extra, target_dir):
    """How the doctests ended: the exit status, each library's results in turn, the
    `Doc-tests` lines and cargo's lines of what failed, and all that was said beside."""
    path = os.path.dirname(RA_JIT) + os.pathsep + os.environ["PATH"]
    env = {**os.environ, **extra, "PATH": path, "CARGO_TARGET_DIR": target_dir}
    run = subprocess.run(["cargo", *command, "test", "--doc", *args], cwd=ws, capture_output=True,
                         text=True, timeout=3600, env=env)
    libraries = [doctest_report("running " + it) for it in re.split(r"(?m)^running ", run.stdout)[1:]]
    # Cargo colors its lines under `CARGO_TERM_COLOR=always`, as CI has it; the text is compared.
    said = re.sub(r"\x1b\[[0-9;]*m", "", run.stderr)
    doc_tests = re.findall(r"(?m)^\s*Doc-tests (\S+)$", said)
    failed = re.findall(r"(?m)^(error: .*|    `-p .*)$", said)
    return (run.returncode, libraries, doc_tests, failed), said


def first_difference(want, got):
    """Where cargo's and ra-jit's first differ, down to a line."""
    if isinstance(want, list) and isinstance(got, list):
        for w, g in zip(want, got):
            if w != g:
                return first_difference(w, g)
        return f"cargo's has {len(want)}, ra-jit's {len(got)}"
    if isinstance(want, str) and isinstance(got, str) and "\n" in want + got:
        return first_difference(want.splitlines(), got.splitlines())
    return f"cargo's {want!r}, ra-jit's {got!r}"


def main():
    wrong = 0
    with tempfile.TemporaryDirectory(prefix="ra-jit-doctestonce-") as scratch:
        ws = os.path.join(scratch, "ws")
        for file, text in WORKSPACE.items():
            os.makedirs(os.path.dirname(os.path.join(ws, file)), exist_ok=True)
            open(os.path.join(ws, file), "w").write(text)
        for name, args, extra in STEPS:
            want, _ = tested(ws, [], args, extra, os.path.join(scratch, "cargo"))
            got, said = tested(ws, ["ra-jit"], args, extra, os.path.join(scratch, "ra-jit"))
            loads = len(re.findall(r"(?m)^load workspace: ", said))
            left = re.findall(r"blaze: (?:the doctests are left to rustdoc|.* is rustc's): .*", said)
            built = sum(int(n) for n in re.findall(r"doctests: (\d+) built by blaze", said))
            parts = ["exit status", "results", "`Doc-tests` lines", "lines of what failed"]
            differs = [f"{part}: {first_difference(w, g)}"[:400] for part, w, g in zip(parts, want, got) if w != g]
            if loads != 1:
                differs.append(f"the workspace was loaded {loads} times")
            if left:
                differs.append(f"not ra-jit's: {left[0][:80]} ... {left[0][-160:]}")
            if not want[1]:
                differs.append("cargo ran no doctest")
            verdict = "WRONG" if differs else "same"
            wrong += bool(differs)
            print(f"  {name:32} exit {got[0]:3}  {len(got[1])} libraries, {built} examples built by ra-jit, "
                  f"{loads} load{'s' if loads != 1 else ''}  {verdict}", flush=True)
            for it in differs:
                print(f"    {it}", flush=True)
            if differs and got[0] not in (0, want[0]):
                print(f"    ra-jit said: {said.strip()[-800:]!r}", flush=True)
    print(f"{len(STEPS) - wrong}/{len(STEPS)} same")
    sys.exit(1 if wrong else 0)


if __name__ == "__main__":
    main()
