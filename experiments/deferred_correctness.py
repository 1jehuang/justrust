#!/usr/bin/env python3
"""Bounded concretely typed deferred-let differential corpus, not full Rust parity.

Exact frozen stock-classified sources mirror tests/deferred.rs. Supported syntax
is let [mut] name: ConcreteType; with definite initialization checked across CFG
paths. Untyped/backward inference and deferred destructuring remain boundaries.
Reuse module_correctness deadlines, bounded captures and workers 1/8 build/check
validation. Native binaries must be supplied, never built by this harness.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "23f5fa45a5b359d70f15360bd99d1cb8fc5e8dc74b1f28371b67a7621f235761"

# Frozen bear stock oracle. full_source is authoritative, with no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "expected": 7,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "fn run()->i32{let x:i32;x=7;x}\nfn main() { if run() == 7 { println!(\"deferred oracle ok\"); } }\n",
    "group": "positive",
    "name": "scalar_first_init",
    "source": "fn run()->i32{let x:i32;x=7;x}"
  },
  {
    "expected": 7,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "fn run()->i32{let x:(i32,bool);x=(7,true);if x.1{x.0}else{0}}\nfn main() { if run() == 7 { println!(\"deferred oracle ok\"); } }\n",
    "group": "positive",
    "name": "tuple_first_init",
    "source": "fn run()->i32{let x:(i32,bool);x=(7,true);if x.1{x.0}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "struct S{n:i32} fn run()->i32{let x:S;x=S{n:7};x.n}\nfn main() { if run() == 7 { println!(\"deferred oracle ok\"); } }\n",
    "group": "positive",
    "name": "struct_first_init",
    "source": "struct S{n:i32} fn run()->i32{let x:S;x=S{n:7};x.n}"
  },
  {
    "expected": 7,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "enum E{A,B} fn run()->i32{let x:E;x=E::B;match x{E::A=>1,E::B=>7}}\nfn main() { if run() == 7 { println!(\"deferred oracle ok\"); } }\n",
    "group": "positive",
    "name": "enum_first_init",
    "source": "enum E{A,B} fn run()->i32{let x:E;x=E::B;match x{E::A=>1,E::B=>7}}"
  },
  {
    "expected": 10,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "fn choose(b:bool)->i32{let x:i32;if b{x=3;}else{x=7;}x} fn run()->i32{choose(true)+choose(false)}\nfn main() { if run() == 10 { println!(\"deferred oracle ok\"); } }\n",
    "group": "positive",
    "name": "if_both_arms",
    "source": "fn choose(b:bool)->i32{let x:i32;if b{x=3;}else{x=7;}x} fn run()->i32{choose(true)+choose(false)}"
  },
  {
    "expected": 10,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "fn choose(b:bool)->i32{let x:i32;match b{true=>{x=3;},false=>{x=7;}}x} fn run()->i32{choose(true)+choose(false)}\nfn main() { if run() == 10 { println!(\"deferred oracle ok\"); } }\n",
    "group": "positive",
    "name": "match_both_arms",
    "source": "fn choose(b:bool)->i32{let x:i32;match b{true=>{x=3;},false=>{x=7;}}x} fn run()->i32{choose(true)+choose(false)}"
  },
  {
    "expected": 10,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let mut x:S;x=S{n:3};let a=take(x);x=S{n:7};a+take(x)}\nfn main() { if run() == 10 { println!(\"deferred oracle ok\"); } }\n",
    "group": "positive",
    "name": "mutable_moved_root_reinit",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let mut x:S;x=S{n:3};let a=take(x);x=S{n:7};a+take(x)}"
  },
  {
    "expected": 10,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "fn run()->i32{let x:i32;x=3;let y={let x:i32;x=7;x};x+y}\nfn main() { if run() == 10 { println!(\"deferred oracle ok\"); } }\n",
    "group": "positive",
    "name": "lexical_shadow",
    "source": "fn run()->i32{let x:i32;x=3;let y={let x:i32;x=7;x};x+y}"
  },
  {
    "expected": 7,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "enum E{A,B} fn run()->i32{let x:E;match x{_=>7}}\nfn main() { if run() == 7 { println!(\"deferred oracle ok\"); } }\n",
    "group": "positive",
    "name": "wildcard_uninitialized",
    "source": "enum E{A,B} fn run()->i32{let x:E;match x{_=>7}}"
  },
  {
    "expected": 7,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "enum E{A} fn run()->i32{let x:E;match x{E::A=>7}}\nfn main() { if run() == 7 { println!(\"deferred oracle ok\"); } }\n",
    "group": "positive",
    "name": "singleton_uninitialized",
    "source": "enum E{A} fn run()->i32{let x:E;match x{E::A=>7}}"
  },
  {
    "expected": "E0384",
    "full_source": "fn run()->i32{let x:i32;x=3;x=7;x}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "immutable_second_assignment",
    "source": "fn run()->i32{let x:i32;x=3;x=7;x}"
  },
  {
    "expected": "E0381",
    "full_source": "fn choose(b:bool)->i32{let x:i32;if b{x=3;}x} fn run()->i32{choose(false)}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "branch_missing_init",
    "source": "fn choose(b:bool)->i32{let x:i32;if b{x=3;}x} fn run()->i32{choose(false)}"
  },
  {
    "expected": "E0381",
    "full_source": "fn run()->i32{let x:i32;x}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "direct_uninitialized_read",
    "source": "fn run()->i32{let x:i32;x}"
  },
  {
    "expected": "E0384",
    "full_source": "fn run()->i32{let x:i32;let mut n=0;while n<2{x=n;n=n+1;}0}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "loop_immutable_assignment",
    "source": "fn run()->i32{let x:i32;let mut n=0;while n<2{x=n;n=n+1;}0}"
  },
  {
    "expected": "E0381",
    "full_source": "struct S{n:i32} fn run()->i32{let mut x:S;x.n=7;x.n}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "projected_write_uninitialized",
    "source": "struct S{n:i32} fn run()->i32{let mut x:S;x.n=7;x.n}"
  },
  {
    "expected": "E0381",
    "full_source": "enum E{A,B} fn run()->i32{let x:E;match x{E::A=>1,E::B=>7}}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "variant_uninitialized",
    "source": "enum E{A,B} fn run()->i32{let x:E;match x{E::A=>1,E::B=>7}}"
  },
  {
    "expected": "E0384",
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let x:S;x=S{n:3};let a=take(x);x=S{n:7};a+take(x)}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "immutable_moved_root_reinit",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let x:S;x=S{n:3};let a=take(x);x=S{n:7};a+take(x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "fn run()->i32{let x;x=7;x}\nfn main() { if run() == 7 { println!(\"deferred oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "untyped_deferred",
    "source": "fn run()->i32{let x;x=7;x}"
  },
  {
    "expected": 7,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "fn run()->i32{let x:_;x=7;x}\nfn main() { if run() == 7 { println!(\"deferred oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "backward_inferred_type",
    "source": "fn run()->i32{let x:_;x=7;x}"
  },
  {
    "expected": 10,
    "expected_stdout": "deferred oracle ok\n",
    "full_source": "fn run()->i32{let (a,b):(i32,i32);(a,b)=(3,7);a+b}\nfn main() { if run() == 10 { println!(\"deferred oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "deferred_tuple_pattern",
    "source": "fn run()->i32{let (a,b):(i32,i32);(a,b)=(3,7);a+b}"
  }
]
''')


def corpus():
    kinds = {"positive": "valid", "negative": "invalid",
             "legalunsupported": "boundary"}
    for fixture in FIXTURES:
        yield Case(fixture["name"], kinds[fixture["group"]],
                   fixture["full_source"], fixture.get("expected_stdout", ""))


def stock_case(case, index, directory, rustc, compile_timeout, run_timeout):
    """Classify independently without treating any native result as the oracle."""
    directory.mkdir()
    source = directory / "input.rs"
    source.write_text(case.source, encoding="utf-8")
    output = directory / "stock"
    result = process([rustc, "--edition=2024", "--crate-name", "reference", "-C",
                      "overflow-checks=yes", source, "-o", output], directory, compile_timeout)
    row = {"record": "stock_case", "index": index, "name": case.name,
           "kind": case.kind, "source": case.source,
           "source_sha256": hashlib.sha256(case.source.encode()).hexdigest(),
           "stock_compile": result, "failures": []}
    if case.kind == "invalid":
        if not rejected(result) or output.exists():
            row["failures"].append("invalid fixture must be rejected by stock without executable")
    elif not accepted(result) or not output.is_file():
        row["failures"].append("valid/boundary fixture must compile with stock")
    else:
        row["stock_run"] = process([output], directory, run_timeout)
        run = row["stock_run"]
        if not accepted(run) or run["stdout"] != case.stdout or run["stderr"]:
            row["failures"].append("stock runtime differs from exact expected stdout/status")
    row["passed"] = not row["failures"]
    row["outcome"] = "stock_classified" if row["passed"] else "failure"
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--rustc", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--stock-only", action="store_true")
    parser.add_argument("--compile-timeout", type=float, default=30)
    parser.add_argument("--run-timeout", type=float, default=5)
    args = parser.parse_args()
    if not args.stock_only and args.binary is None:
        parser.error("--binary is required unless --stock-only")
    if not 0 < args.compile_timeout <= 120 or not 0 < args.run_timeout <= 30:
        parser.error("timeouts must be positive and bounded by 120 compile / 30 run seconds")
    rustc = args.rustc.absolute()  # Preserve a rustup proxy's executable name.
    if not rustc.is_file():
        parser.error("--rustc must name an existing executable")
    binary = args.binary.resolve(strict=True) if args.binary else None
    cases = list(corpus())
    assert len(cases) == 20 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-deferred-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "deferred-typed-v1",
                "case_count": len(cases),
                "case_counts": {kind: sum(c.kind == kind for c in cases)
                                for kind in ("valid", "invalid", "boundary")},
                "stock_only": args.stock_only,
                "workers": [] if args.stock_only else [1, 8],
                "modes": [] if args.stock_only else ["build", "check"], "edition": 2024,
                "reference_modes": ["build", "run"], "rustc_path": str(rustc),
                "reference": version["stdout"],
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest() if binary else None,
                "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                "shared_harness_sha256": hashlib.sha256(Path(__file__).with_name("module_correctness.py").read_bytes()).hexdigest(),
                "corpus_sha256": corpus_hash,
                "acceptance_corpus_sha256": ACCEPTANCE_CORPUS_SHA256,
                "timing_scope": "process/deadline/capture overhead included, not a compiler latency benchmark",
                "note": "legal unsupported boundaries are not invalid Rust or parity"})
        totals = {"accepted_parity": 0, "invalid_rejected": 0, "unsupported_boundary": 0,
                  "stock_classified": 0, "failure": 0}
        for index, case in enumerate(cases):
            if args.stock_only:
                row = stock_case(case, index, directory / str(index), rustc,
                                 args.compile_timeout, args.run_timeout)
            else:
                row = run_case(case, index, directory / str(index), binary, rustc,
                               args.compile_timeout, args.run_timeout)
            row["expected_stdout"] = case.stdout
            record(row)
            totals[row["outcome"]] += 1
            print(f"{case.name}: {row['outcome']}", flush=True)
        record({"record": "summary", **totals, "passed": totals["failure"] == 0})
        if totals["failure"]:
            raise SystemExit(f"{totals['failure']} deferred-let discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
