#!/usr/bin/env python3
"""Native off/on correctness gate, not a benchmark. Requires a newly built jr-rustc.

Each edit is compiled with persistent incremental state and with a fresh cache,
for both flag settings. Compare complete JSON diagnostics and exit status, and
check independent expected lint/error codes so two equally broken runs fail.
Nothing runs at import time. Unit tests use no compiler.
"""
import argparse
from dataclasses import dataclass
import json
from pathlib import Path
import subprocess
import tempfile

FLAG = "-Zjr-empty-lint-level-fast-path="


@dataclass(frozen=True)
class Step:
    name: str
    source: str
    codes: tuple[str, ...] = ()
    fails: bool = False
    flags: tuple[str, ...] = ()


def steps():
    unused = "pub fn f() { let unused = 1; }\n"
    yield Step("empty", "pub fn f() {}\n")
    yield Step("crate-forbid", "#![forbid(unused_variables)]\n" + unused,
               ("unused_variables",), True)
    yield Step("parent-forbid", "#[forbid(unused_variables)]\npub mod p { " + unused + "}\n",
               ("unused_variables",), True)
    yield Step("command-deny", unused, ("unused_variables",), True,
               ("-Dunused-variables",))
    yield Step("attribute-empty", unused, ("unused_variables",))
    yield Step("attribute-added", "#[allow(unused_variables)]\n" + unused)
    yield Step("attribute-removed", unused, ("unused_variables",))
    yield Step("body-attribute", "pub fn f() { #[allow(unused_variables)] let unused = 1; }\n")
    yield Step("expect-fulfilled", "#[expect(unused_variables)]\n" + unused)
    yield Step("expect-unfulfilled", "#[expect(unused_variables)]\npub fn f() {}\n",
               ("unfulfilled_lint_expectations",))
    yield Step("expect-removed", "pub fn f() {}\n")
    yield Step("crate-expect", "#![expect(unused_variables)]\n" + unused)
    yield Step("parent-expect", "#[expect(unused_variables)]\npub mod p { " + unused + "}\n")
    yield Step("nested-owners", "#![forbid(unused_variables)]\npub fn outer() { fn inner() { let unused = 1; } inner(); }\n",
               ("unused_variables",), True)
    yield Step("forbid-override", "#![forbid(unused_variables)]\n#[allow(unused_variables)]\n" + unused,
               ("E0453",), True)
    yield Step("macro-empty", "macro_rules! make { () => { pub fn f() { let unused = 1; } }; } make!();\n",
               ("unused_variables",))
    yield Step("macro-attribute", "macro_rules! make { () => { #[allow(unused_variables)] pub fn f() { let unused = 1; } }; } make!();\n")
    yield Step("macro-reverted", "macro_rules! make { () => { pub fn f() { let unused = 1; } }; } make!();\n",
               ("unused_variables",))
    yield Step("unknown-lint", "#[allow(not_a_real_lint)]\npub fn f() {}\n", ("unknown_lints",))
    yield Step("type-error", "pub fn f() { let _: u8 = \"bad\"; }\n", ("E0308",), True)
    yield Step("parse-error", "pub fn f( {\n", (), True)
    yield Step("recovery", "pub fn f() {}\n")


def diagnostics(stderr):
    """Keep complete diagnostic trees, including suggestions, spans and rendering."""
    return [json.loads(line) for line in stderr.splitlines() if line.strip()]


def check_expected(step, result):
    status, diags = result
    assert status == (1 if step.fails else 0), (step.name, "exit", status, diags)
    assert any(d.get("level") == "error" for d in diags) == step.fails, (step.name, diags)
    codes = [d["code"]["code"] for d in diags if d.get("code")]
    # Other codes are forbidden except the extra unused-variable diagnostic
    # accompanying an illegal attempt to lower an inherited forbid.
    expected = list(step.codes)
    if step.name == "forbid-override":
        codes = [code for code in codes if code != "unused_variables"]
    assert sorted(codes) == sorted(expected), (step.name, "codes", codes, expected, diags)


def command(rustc, root, cache, enabled, extra=()):
    return [str(rustc), str(root / "main.rs"), "--crate-name", "lint_accept",
            "--crate-type=lib", "--edition=2024", "--emit=metadata",
            "--error-format=json", "--color=never", "-o", str(root / "out.rmeta"),
            "-Cincremental=" + str(cache), FLAG + ("yes" if enabled else "no"), *extra]


def run(rustc, root):
    for index, step in enumerate(steps()):
        (root / "main.rs").write_text(step.source)
        results = []
        for enabled in (False, True):
            for fresh in (False, True):
                cache = root / (f"fresh-{index}-{enabled}" if fresh else f"incremental-{enabled}")
                proc = subprocess.run(command(rustc, root, cache, enabled, step.flags),
                                      capture_output=True, text=True, timeout=120)
                assert not proc.stdout.strip(), (step.name, "unexpected stdout", proc.stdout)
                result = (proc.returncode, diagnostics(proc.stderr))
                check_expected(step, result)
                results.append(result)
        assert all(result == results[0] for result in results), (step.name, results)
        print(f"PASS {step.name}: off/on, fresh/incremental", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("rustc", type=Path, help="new native jr-rustc binary, not justrust")
    args = parser.parse_args()
    rustc = args.rustc.resolve(strict=True)
    with tempfile.TemporaryDirectory(prefix="jr-empty-lint-accept-") as directory:
        run(rustc, Path(directory))
    print("Native acceptance passed (diagnostics and status only, no speed claim).")


if __name__ == "__main__":
    main()
