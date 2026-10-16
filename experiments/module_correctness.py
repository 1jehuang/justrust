#!/usr/bin/env python3
"""Bounded inline-module differential checks, separate from the compiler.

Supported valid cases must compile and match stock stdout and process status at
both worker counts. Invalid Rust must be rejected by both compilers without an
output. Explicit unsupported features are recorded as boundaries, never parity.
Only the Python standard library is used. Each command has a deadline, output is
file-backed and read with a cap, and at most one child process runs at a time.
"""

import argparse
from dataclasses import dataclass, field
import hashlib
import json
import os
from pathlib import Path
import random
import re
import signal
import subprocess
import tempfile
import time


@dataclass(frozen=True)
class Case:
    name: str
    kind: str
    source: str
    stdout: str = ""
    files: dict = field(default_factory=dict)


def valid_cases(rng, count):
    yield Case("external_module", "valid", 'mod external; fn main() { external::f(); }',
               files={"external.rs": "pub fn f() {}"})
    for index in range(count):
        a, b, c = (rng.randint(1, 31) for _ in range(3))
        depth = rng.randint(4, 14) * 2
        suffix = f"_{index}_{rng.randrange(1000000)}"
        left, right = "left" + suffix, "right" + suffix
        source = f'''fn root() -> i32 {{ {a} }}
fn main() {{
    if alias({depth}) && !alias({depth + 1}) {{ println!("recursion"); }}
    if imported::sum() == {a + b + c} {{ println!("paths"); }}
    if {left}::child::value() == {b + c} {{ println!("private ancestors"); }}
    let value = 8;
    let imported = 9;
    if crate::value() == {a} && imported::value() == {c} && value + imported == 17 {{
        println!("qualified shadowing");
    }}
    if chained() == {c} {{ println!("forward aliases"); }}
    println!("done {index}");
}}
use self::first_alias as chained;
use crate::{left}::nested::value as first_alias;
use crate::{left}::nested as imported;
use crate::{right}::even as alias;
fn value() -> i32 {{ {a} }}
mod {left} {{
    fn value() -> i32 {{ {b} }}
    mod hidden {{ pub fn value() -> i32 {{ {c} }} }}
    pub mod child {{
        pub fn value() -> i32 {{ super::value() + super::hidden::value() }}
    }}
    pub mod nested {{
        use super::value as parent;
        pub fn value() -> i32 {{ {c} }}
        pub fn sum() -> i32 {{ self::value() + parent() + crate::root() }}
    }}
    pub fn odd(n: i32) -> bool {{
        if n == 0 {{ false }} else {{ crate::{right}::even(n - 1) }}
    }}
}}
mod {right} {{
    use crate::{left}::odd as next;
    pub fn even(n: i32) -> bool {{ if n == 0 {{ true }} else {{ next(n - 1) }} }}
    pub fn main() {{ println!("wrong entry"); }}
}}
'''
        yield Case(f"generated_{index}", "valid", source,
                   "recursion\npaths\nprivate ancestors\nqualified shadowing\nforward aliases\n"
                   f"done {index}\n")
    yield Case("relative_imports", "valid", '''
mod parent {
    mod leaf { pub fn value() -> i32 { 17 } }
    use leaf::value as local;
    pub mod child {
        use super::leaf as sibling;
        use super::leaf::value as alias;
        pub fn run() -> i32 { sibling::value() + alias() + super::local() }
    }
}
fn main() { if parent::child::run() == 51 { println!("relative"); } }
''', "relative\n")
    yield Case("module_alias_chain", "valid", '''
use self::first as last;
use self::target as first;
mod target { pub fn f() -> i32 { 37 } }
fn main() { if last::f() == 37 { println!("chain"); } }
''', "chain\n")
    yield Case("same_spelling_namespaces", "valid", '''
mod same { pub fn value() -> i32 { 3 } }
fn same() -> i32 { 7 }
mod a { pub fn f() -> i32 { 13 } }
use a::f as a;
fn main() { if same() + same::value() + a() + a::f() == 36 { println!("namespaces"); } }
''', "namespaces\n")
    yield Case("keyword_module_imports", "valid", '''
use crate as root;
mod a {
    use self as me;
    fn value() -> i32 { 17 }
    pub fn run() -> i32 { me::value() }
}
fn value() -> i32 { 19 }
fn main() { if a::run() + root::value() == 36 { println!("keyword imports"); } }
''', "keyword imports\n")
    yield Case("public_module_private_function", "valid", '''
mod a { pub mod same { pub fn f() -> i32 { 3 } } fn same() -> i32 { 99 } }
use a::same as alias;
fn main() { if alias::f() == 3 { println!("public module"); } }
''', "public module\n")
    yield Case("public_function_private_module", "valid", '''
mod a { mod same {} pub fn same() -> i32 { 7 } }
use a::same as alias;
fn main() { if alias() == 7 { println!("public function"); } }
''', "public function\n")
    yield Case("unaliased_public_function_private_module", "valid", '''
mod a { mod same {} pub fn same() -> i32 { 11 } }
use a::same;
fn main() { if same() == 11 { println!("unaliased function"); } }
''', "unaliased function\n")


def invalid_cases():
    sources = [
        ("duplicate_module", "mod a {} mod a {} fn main() {}"),
        ("duplicate_nested_function", "mod a { fn f() {} fn f() {} } fn main() {}"),
        ("duplicate_function_import", "mod a { pub fn f() {} } use a::f; use a::f; fn main() {}"),
        ("function_import_collision", "mod a { pub fn f() {} } use a::f; fn f() {} fn main() {}"),
        ("module_import_collision", "mod a {} mod b {} use a as b; fn main() {}"),
        ("private_sibling_function", "mod a { fn f() {} } mod b { pub fn g() { crate::a::f(); } } fn main() {}"),
        ("private_child_function", "mod a { fn f() {} } fn main() { a::f(); }"),
        ("private_nested_module", "mod a { mod hidden { pub fn f() {} } } fn main() { a::hidden::f(); }"),
        ("private_import_target", "mod a { fn f() {} } use a::f as alias; fn main() {}"),
        ("private_alias", "mod a { pub fn f() {} } mod b { use crate::a::f as alias; } fn main() { b::alias(); }"),
        ("missing_unused_import", "use crate::missing; fn main() {}"),
        ("missing_nested_unused_import", "mod a { use self::missing as unused; } fn main() {}"),
        ("missing_imported_child", "mod a {} use a::missing as unused; fn main() {}"),
        ("import_cycle", "use self::b as a; use self::a as b; fn main() {}"),
        ("root_super", "fn main() { super::f(); } fn f() {}"),
        ("too_many_super", "mod a { pub fn f() { super::super::g(); } } fn g() {} fn main() {}"),
        ("missing_qualified_function", "mod a {} fn main() { a::missing(); }"),
        ("function_as_module", "fn a() {} fn main() { a::f(); }"),
        ("module_as_function", "mod a {} fn main() { a(); }"),
        ("local_shadows_function", "fn f() {} fn main() { let f = 1; f(); }"),
        ("local_shadows_import", "mod a { pub fn f() {} } use a::f; fn main() { let f = 1; f(); }"),
        ("nested_main_only", "mod a { pub fn main() {} }"),
        ("invalid_unused_body", "mod a { fn unused() { missing(); } } fn main() {}"),
        ("wrong_argument", "mod a { pub fn f(x: i32) {} } fn main() { a::f(true); }"),
        ("unterminated_module", "mod a { fn f() {} fn main() {}"),
        ("missing_module_name", "mod {} fn main() {}"),
        ("missing_use_target", "use ; fn main() {}"),
        ("missing_alias", "mod a {} use a as ; fn main() {}"),
        ("private_function_public_module", "mod a { pub mod same {} fn same() {} } use a::same as alias; fn main() { alias(); }"),
        ("private_module_public_function", "mod a { mod same { pub fn f() {} } pub fn same() {} } use a::same as alias; fn main() { alias::f(); }"),
    ]
    for name, text in sources:
        yield Case(name, "invalid", text)


def boundary_cases():
    for name, text in [
        ("imported_main_only", "mod a { pub fn entry() {} } use a::entry as main;"),
        ("grouped_use", "mod a { pub fn f() {} pub fn g() {} } use a::{f, g}; fn main() { f(); g(); }"),
        ("glob_use", "mod a { pub fn f() {} } use a::*; fn main() { f(); }"),
        ("restricted_module_visibility", "pub(crate) mod a {} fn main() {}"),
        ("restricted_function_visibility", "mod a { pub(super) fn f() {} } fn main() { a::f(); }"),
        ("public_use", "mod a { pub fn f() {} } pub use a::f; fn main() { f(); }"),
    ]:
        yield Case(name, "boundary", text)


MAX_CAPTURE = 65536


def process(argv, directory, timeout):
    """Kill and reap a process group at its deadline, with bounded RAM capture."""
    start = time.perf_counter_ns()
    with tempfile.TemporaryFile(dir=directory) as stdout, tempfile.TemporaryFile(dir=directory) as stderr:
        with subprocess.Popen([str(arg) for arg in argv], stdin=subprocess.DEVNULL,
                              stdout=stdout, stderr=stderr, start_new_session=True) as child:
            timed_out = False
            try:
                status = child.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                timed_out = True
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                status = child.wait()
        stdout.seek(0)
        stderr.seek(0)
        a, b = stdout.read(MAX_CAPTURE + 1), stderr.read(MAX_CAPTURE + 1)
        return {"status": status, "timed_out": timed_out,
                "wall_ns": time.perf_counter_ns() - start,
                "truncated": len(a) > MAX_CAPTURE or len(b) > MAX_CAPTURE,
                "stdout_sha256": hashlib.sha256(a).hexdigest(),
                "stderr_sha256": hashlib.sha256(b).hexdigest(),
                "stdout": a[:MAX_CAPTURE].decode("utf-8", errors="replace"),
                "stderr": b[:MAX_CAPTURE].decode("utf-8", errors="replace")}


def completed(result):
    return not result["timed_out"] and not result["truncated"]


def accepted(result):
    return completed(result) and result["status"] == 0


def rejected(result):
    # Both rustc and jr-rust document ordinary compile errors as exit 1.
    # A panic, crash, timeout, or signal is not a semantic rejection.
    return completed(result) and result["status"] == 1 and bool(result["stderr"])


def run_case(case, index, directory, binary, rustc, compile_timeout, run_timeout):
    start = time.perf_counter_ns()
    directory.mkdir()
    input_path = directory / "input.rs"
    input_path.write_text(case.source, encoding="utf-8")
    for name, text in case.files.items():
        (directory / name).write_text(text, encoding="utf-8")
    reference = directory / "stock"
    assert not reference.exists()
    stock = process([rustc, "--edition=2024", "--crate-name", "reference", "-C",
                     "overflow-checks=yes", input_path, "-o", reference], directory, compile_timeout)
    row = {"record": "case", "index": index, "name": case.name, "kind": case.kind,
           "source": case.source, "files": case.files,
           "source_bytes": len(case.source.encode()),
           "functions": len(re.findall(r"\bfn\s+[A-Za-z_]", case.source)),
           "modules": len(re.findall(r"\bmod\s+[A-Za-z_]", case.source)),
           "source_sha256": hashlib.sha256(case.source.encode()).hexdigest(),
           "stock_compile": stock, "workers": [], "failures": []}
    if case.kind == "invalid":
        if not rejected(stock) or reference.exists():
            row["failures"].append("invalid fixture must be rejected by stock without output")
    elif not accepted(stock) or not reference.is_file():
        row["failures"].append("valid/boundary fixture must compile with stock")
    else:
        row["stock_run"] = process([reference], directory, run_timeout)
        if not accepted(row["stock_run"]) or row["stock_run"]["stdout"] != case.stdout or row["stock_run"]["stderr"]:
            row["failures"].append("stock runtime did not produce fixture's expected stdout/status")
    for workers in (1, 8):
        output = directory / f"native-{workers}"
        assert not output.exists()
        command = [binary, input_path, "-o", output, "--workers", workers]
        native = process(command, directory, compile_timeout)
        worker = {"workers": workers, "compile": native, "output_exists": output.exists()}
        row["workers"].append(worker)
        if case.kind == "valid":
            if not accepted(native) or not output.is_file():
                row["failures"].append(f"supported case rejected with workers={workers}")
            else:
                worker["run"] = process([output], directory, run_timeout)
                reference_run = row.get("stock_run", {})
                if not completed(worker["run"]) or any(worker["run"].get(key) != reference_run.get(key)
                                                       for key in ("stdout_sha256", "stderr_sha256", "status")):
                    row["failures"].append(f"runtime mismatch with workers={workers}")
        elif not rejected(native) or output.exists():
            row["failures"].append(f"{case.kind} must reject without output with workers={workers}")
        before_check = set(directory.iterdir())
        worker["check"] = process([binary, input_path, "--workers", workers,
                                   "--check"], directory, compile_timeout)
        if set(directory.iterdir()) != before_check or (not accepted(worker["check"]) if case.kind == "valid" else not rejected(worker["check"])):
            row["failures"].append(f"check-mode contract failed with workers={workers}")
    row["passed"] = not row["failures"]
    row["outcome"] = ("unsupported_boundary" if case.kind == "boundary" else
                      "accepted_parity" if case.kind == "valid" else "invalid_rejected") if row["passed"] else "failure"
    row["wall_ns"] = time.perf_counter_ns() - start
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--rustc", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cases", type=int, default=16, help="generated valid cases, in addition to fixed cases")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--compile-timeout", type=float, default=30)
    parser.add_argument("--run-timeout", type=float, default=5)
    args = parser.parse_args()
    if not 1 <= args.cases <= 1000:
        parser.error("--cases must be 1..1000")
    if not 0 < args.compile_timeout <= 120 or not 0 < args.run_timeout <= 30:
        parser.error("timeouts must be positive and bounded by 120 compile / 30 run seconds")
    binary = args.binary.resolve(strict=True)
    # Keep rustup's rustc symlink name rather than resolving it to rustup.
    rustc = args.rustc.absolute()
    if not rustc.is_file():
        parser.error("--rustc must name an existing executable")
    cases = [*valid_cases(random.Random(args.seed), args.cases), *invalid_cases(), *boundary_cases()]
    scratch = os.environ.get("JCODE_SCRATCH_DIR")
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-module-differential-", dir=scratch) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "seed": args.seed, "generated_cases": args.cases,
                "workers": [1, 8], "reference": version["stdout"],
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                "timing_scope": "wall_ns includes process launch, timeout wait polling and capture, not a compiler latency benchmark",
                "note": "boundaries are unsupported features, not parity"})
        totals = {"accepted_parity": 0, "invalid_rejected": 0, "unsupported_boundary": 0, "failure": 0}
        for index, case in enumerate(cases):
            row = run_case(case, index, directory / str(index), binary, rustc,
                           args.compile_timeout, args.run_timeout)
            record(row)
            totals[row["outcome"]] += 1
            print(f"{case.name}: {row['outcome']}", flush=True)
        record({"record": "summary", **totals, "passed": totals["failure"] == 0})
        if totals["failure"]:
            raise SystemExit(f"{totals['failure']} module discrepancies, exact sources and diagnostics in {args.output}")


if __name__ == "__main__":
    main()
