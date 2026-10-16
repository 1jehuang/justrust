#!/usr/bin/env python3
"""Bounded fixed-array differential corpus, not full Rust parity.

Exact stock-classified source bytes and stdout mirror bear's array acceptance
corpus. First slice: fixed arrays of at most 4096 elements, literals/repeats,
concrete types, literal indexing and indexed writes with ownership checks.
Legal unsupported dynamic indexing, patterns, slices, methods and backward
inference remain boundaries. Reuse module_correctness deadlines, bounded capture
and supplied-native workers 1/8 check/build. Never build a native binary here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "5d36478f8c11ac955274b518f6c6657fe2f2ecde768ac7a5538b07921a1c4bd5"

# Authoritative stock oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "group": "positive",
    "name": "array_literal",
    "source": "fn run()->i32{let a=[3,7];a[0]*10+a[1]}",
    "expected": 37,
    "full_source": "fn run()->i32{let a=[3,7];a[0]*10+a[1]}\nfn main() { if run() == 37 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "positive",
    "name": "copy_repeat",
    "source": "fn run()->i32{let a=[7;3];a[0]+a[1]+a[2]}",
    "expected": 21,
    "full_source": "fn run()->i32{let a=[7;3];a[0]+a[1]+a[2]}\nfn main() { if run() == 21 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "positive",
    "name": "typed_empty_literal",
    "source": "struct R{a:[i32;0]} enum E{A([i32;0])} fn make()->[i32;0]{[]} fn returned()->[i32;0]{return [];} fn take(a:[i32;0])->i32{1} fn run()->i32{let a:[i32;0]=[];let r=R{a:[]};let e=E::A([]);take(a)+take([])+take(make())+take(returned())+take(r.a)+match e{E::A(a)=>take(a)}}",
    "expected": 6,
    "full_source": "struct R{a:[i32;0]} enum E{A([i32;0])} fn make()->[i32;0]{[]} fn returned()->[i32;0]{return [];} fn take(a:[i32;0])->i32{1} fn run()->i32{let a:[i32;0]=[];let r=R{a:[]};let e=E::A([]);take(a)+take([])+take(make())+take(returned())+take(r.a)+match e{E::A(a)=>take(a)}}\nfn main() { if run() == 6 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "positive",
    "name": "zero_copy_whole",
    "source": "fn consume(a:[i32;0])->i32{3} fn run()->i32{let a=[7;0];consume(a)+consume(a)}",
    "expected": 6,
    "full_source": "fn consume(a:[i32;0])->i32{3} fn run()->i32{let a=[7;0];consume(a)+consume(a)}\nfn main() { if run() == 6 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "positive",
    "name": "noncopy_repeat_zero_effect",
    "source": "struct S{n:i32} fn make()->S{println!(\"once\");S{n:3}} fn run()->i32{let a=[make();0];7}",
    "expected": "once\n7",
    "full_source": "struct S{n:i32} fn make()->S{println!(\"once\");S{n:3}} fn run()->i32{let a=[make();0];7}\nfn main() { if run() == 7 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "once\narray oracle ok\n"
  },
  {
    "group": "positive",
    "name": "noncopy_repeat_one_effect",
    "source": "struct S{n:i32} fn make()->S{println!(\"once\");S{n:7}} fn run()->i32{let a=[make();1];a[0].n}",
    "expected": "once\n7",
    "full_source": "struct S{n:i32} fn make()->S{println!(\"once\");S{n:7}} fn run()->i32{let a=[make();1];a[0].n}\nfn main() { if run() == 7 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "once\narray oracle ok\n"
  },
  {
    "group": "positive",
    "name": "copy_field_noncopy_element",
    "source": "struct S{n:i32} fn run()->i32{let a=[S{n:3},S{n:7}];let _=a[0];a[0].n+a[1].n+a[0].n}",
    "expected": 13,
    "full_source": "struct S{n:i32} fn run()->i32{let a=[S{n:3},S{n:7}];let _=a[0];a[0].n+a[1].n+a[0].n}\nfn main() { if run() == 13 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutable_scalar_index_assignment",
    "source": "fn run()->i32{let mut a=[3,4];a[1]=7;a[0]*10+a[1]}",
    "expected": 37,
    "full_source": "fn run()->i32{let mut a=[3,4];a[1]=7;a[0]*10+a[1]}\nfn main() { if run() == 37 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutable_noncopy_index_assignment",
    "source": "struct S{n:i32} fn run()->i32{let mut a=[S{n:3},S{n:4}];a[1]=S{n:7};a[0].n*10+a[1].n}",
    "expected": 37,
    "full_source": "struct S{n:i32} fn run()->i32{let mut a=[S{n:3},S{n:4}];a[1]=S{n:7};a[0].n*10+a[1].n}\nfn main() { if run() == 37 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "negative",
    "name": "wildcard_uninitialized_index",
    "source": "struct S{n:i32} fn run()->i32{let a:[S;2];let _=a[0];7}",
    "expected": "E0381",
    "full_source": "struct S{n:i32} fn run()->i32{let a:[S;2];let _=a[0];7}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "wildcard_moved_index",
    "source": "struct S{n:i32} fn consume(a:[S;2])->i32{3} fn run()->i32{let a=[S{n:1},S{n:2}];let n=consume(a);let _=a[0];n+4}",
    "expected": "E0382",
    "full_source": "struct S{n:i32} fn consume(a:[S;2])->i32{3} fn run()->i32{let a=[S{n:1},S{n:2}];let n=consume(a);let _=a[0];n+4}\nfn main() { run(); }\n"
  },
  {
    "group": "positive",
    "name": "nested_array_values",
    "source": "fn run()->i32{let a:[i32;2]=[3,7];let b:[[i32;2];2]=[a;2];a[0]*10+b[1][1]}",
    "expected": 37,
    "full_source": "fn run()->i32{let a:[i32;2]=[3,7];let b:[[i32;2];2]=[a;2];a[0]*10+b[1][1]}\nfn main() { if run() == 37 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "positive",
    "name": "literal_source_order",
    "source": "fn first()->i32{println!(\"first\");3} fn second()->i32{println!(\"second\");7} fn run()->i32{let a=[first(),second()];a[0]*10+a[1]}",
    "expected": "first\nsecond\n37",
    "full_source": "fn first()->i32{println!(\"first\");3} fn second()->i32{println!(\"second\");7} fn run()->i32{let a=[first(),second()];a[0]*10+a[1]}\nfn main() { if run() == 37 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "first\nsecond\narray oracle ok\n"
  },
  {
    "group": "negative",
    "name": "noncopy_index_move",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let a=[S{n:7}];take(a[0])}",
    "expected": "E0508",
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let a=[S{n:7}];take(a[0])}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "noncopy_repeat_two",
    "source": "struct S{n:i32} fn run()->i32{let s=S{n:7};let a=[s;2];0}",
    "expected": "E0277",
    "full_source": "struct S{n:i32} fn run()->i32{let s=S{n:7};let a=[s;2];0}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "repeat_zero_consumes",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let s=S{n:7};let a=[s;0];take(s)}",
    "expected": "E0382",
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let s=S{n:7};let a=[s;0];take(s)}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "repeat_one_consumes",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let s=S{n:7};let a=[s;1];take(s)}",
    "expected": "E0382",
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let s=S{n:7};let a=[s;1];take(s)}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "zero_noncopy_whole_move",
    "source": "struct S{n:i32} fn consume(a:[S;0])->i32{3} fn run()->i32{let a:[S;0]=[];let n=consume(a);n+consume(a)}",
    "expected": "E0382",
    "full_source": "struct S{n:i32} fn consume(a:[S;0])->i32{3} fn run()->i32{let a:[S;0]=[];let n=consume(a);n+consume(a)}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "immutable_index_assignment",
    "source": "fn run()->i32{let a=[3,4];a[1]=7;a[1]}",
    "expected": "E0594",
    "full_source": "fn run()->i32{let a=[3,4];a[1]=7;a[1]}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "wrong_array_length",
    "source": "fn run()->i32{let a:[i32;2]=[7];a[0]}",
    "expected": "E0308",
    "full_source": "fn run()->i32{let a:[i32;2]=[7];a[0]}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "i32_runtime_index",
    "source": "fn index()->i32{1} fn run()->i32{let a=[3,7];a[index()]}",
    "expected": "E0277",
    "full_source": "fn index()->i32{1} fn run()->i32{let a=[3,7];a[index()]}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "constant_out_of_bounds",
    "source": "fn run()->i32{let a=[3,7];a[2]}",
    "expected": null,
    "full_source": "fn run()->i32{let a=[3,7];a[2]}\nfn main() { run(); }\n"
  },
  {
    "group": "positive",
    "name": "dynamic_usize_index",
    "source": "fn index()->usize{1} fn run()->i32{let a=[3,7];a[index()]}",
    "expected": 7,
    "full_source": "fn index()->usize{1} fn run()->i32{let a=[3,7];a[index()]}\nfn main() { if run() == 7 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "array_pattern",
    "source": "fn run()->i32{let [a,b]=[3,7];a*10+b}",
    "expected": 37,
    "full_source": "fn run()->i32{let [a,b]=[3,7];a*10+b}\nfn main() { if run() == 37 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "array_slice",
    "source": "fn run()->i32{let a=[3,7];let s=&a[..];s[1]}",
    "expected": 7,
    "full_source": "fn run()->i32{let a=[3,7];let s=&a[..];s[1]}\nfn main() { if run() == 7 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "array_len_method",
    "source": "fn run()->i32{let a=[3,7];if a.len()==2{7}else{0}}",
    "expected": 7,
    "full_source": "fn run()->i32{let a=[3,7];if a.len()==2{7}else{0}}\nfn main() { if run() == 7 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "negative",
    "name": "empty_literal_without_context",
    "source": "fn run()->i32{let a=[];7}",
    "expected": "E0282",
    "full_source": "fn run()->i32{let a=[];7}\nfn main() { run(); }\n"
  },
  {
    "group": "legalunsupported",
    "name": "later_inferred_empty_literal",
    "source": "fn take(a:[i32;0])->i32{7} fn run()->i32{let a=[];take(a)}",
    "expected": 7,
    "full_source": "fn take(a:[i32;0])->i32{7} fn run()->i32{let a=[];take(a)}\nfn main() { if run() == 7 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
  },
  {
    "group": "negative",
    "name": "noncopy_indexed_field_move",
    "source": "struct S{n:i32} struct R{inner:S} fn take(s:S)->i32{s.n} fn run()->i32{let a=[R{inner:S{n:7}}];take(a[0].inner)}",
    "expected": "E0508",
    "full_source": "struct S{n:i32} struct R{inner:S} fn take(s:S)->i32{s.n} fn run()->i32{let a=[R{inner:S{n:7}}];take(a[0].inner)}\nfn main() { run(); }\n"
  },
  {
    "group": "positive",
    "name": "assignment_rhs_initializes_root",
    "source": "fn run()->i32{let mut a:[i32;1];a[0]={a=[1];7};a[0]}",
    "expected": 7,
    "full_source": "fn run()->i32{let mut a:[i32;1];a[0]={a=[1];7};a[0]}\nfn main() { if run() == 7 { println!(\"array oracle ok\"); } }\n",
    "expected_stdout": "array oracle ok\n"
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
    assert len(cases) == 30 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-array-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "array-fixed-v1",
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
            raise SystemExit(f"{totals['failure']} fixed-array discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
