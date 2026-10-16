#!/usr/bin/env python3
"""Bounded B2 reference-call differential corpus, not full Rust compatibility.

Exact frozen stock-classified source bytes/stdout mirror bear's B2 acceptance
corpus. Only the approved no-escape reference parameter and expected-reborrow
slice is claimed. Legal unsupported reference escapes/aggregates/nesting/rvalues
remain explicit boundaries. Valid runs must exit zero, exact stdout, empty stderr.
Reuse module_correctness Case/run_case/process, deadline-bound file-backed
capture and supplied-native workers 1/8 check/build. Never build a binary here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "db2c825e38382a9b2a124aa6e1ffa021264167758b0f8a7dfe28506f16522d11"

# Original immutable acceptance artifact hash retained above.
# E1 native-pass promotion: reference_return_escape is now positive; source/stdout/value unchanged.
# Authoritative bear source bytes. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn read(r:&i32)->i32{*r} fn run()->i32{let x=7;read(&x)}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "shared_reference_parameter",
    "source": "fn read(r:&i32)->i32{*r} fn run()->i32{let x=7;read(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn write(r:&mut i32){*r=7;} fn run()->i32{let mut x=3;write(&mut x);x}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "immutable_mutable_parameter_carrier",
    "source": "fn write(r:&mut i32){*r=7;} fn run()->i32{let mut x=3;write(&mut x);x}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn bump(r:&mut i32){*r=*r+2;} fn run()->i32{let mut x=3;let r=&mut x;bump(r);bump(r);*r}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "repeated_calls_implicit_reborrow",
    "source": "fn bump(r:&mut i32){*r=*r+2;} fn run()->i32{let mut x=3;let r=&mut x;bump(r);bump(r);*r}"
  },
  {
    "expected": 14,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn sum(a:&i32,b:&i32)->i32{*a+*b} fn run()->i32{let x=7;let r=&x;sum(r,r)}\nfn main() { if run() == 14 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "shared_alias_arguments",
    "source": "fn sum(a:&i32,b:&i32)->i32{*a+*b} fn run()->i32{let x=7;let r=&x;sum(r,r)}"
  },
  {
    "expected": 13,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "struct S{a:i32,b:i32} fn change(a:&mut i32,b:&mut i32){*a=*a+1;*b=*b+2;} fn run()->i32{let mut s=S{a:3,b:7};change(&mut s.a,&mut s.b);s.a+s.b}\nfn main() { if run() == 13 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "disjoint_mutable_field_arguments",
    "source": "struct S{a:i32,b:i32} fn change(a:&mut i32,b:&mut i32){*a=*a+1;*b=*b+2;} fn run()->i32{let mut s=S{a:3,b:7};change(&mut s.a,&mut s.b);s.a+s.b}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn bump(r:&mut i32){*r=*r+4;} fn run()->i32{let mut x=3;let r=&mut x;bump(&mut *r);*r}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "explicit_call_reborrow",
    "source": "fn bump(r:&mut i32){*r=*r+4;} fn run()->i32{let mut x=3;let r=&mut x;bump(&mut *r);*r}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn bump(r:&mut i32){let q=r;*q=*q+4;} fn run()->i32{let mut x=3;let r=&mut x;bump(r);*r}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "formal_mutable_handle_move",
    "source": "fn bump(r:&mut i32){let q=r;*q=*q+4;} fn run()->i32{let mut x=3;let r=&mut x;bump(r);*r}"
  },
  {
    "expected": 10,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn change(p:&mut i32)->i32{let mut r=p;let mut y=7;r=&mut y;*r} fn run()->i32{let mut x=3;let n=change(&mut x);x+n}\nfn main() { if run() == 10 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "formal_handle_rebinding",
    "source": "fn change(p:&mut i32)->i32{let mut r=p;let mut y=7;r=&mut y;*r} fn run()->i32{let mut x=3;let n=change(&mut x);x+n}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn leaf(r:&mut i32){*r=7;} fn forward(r:&mut i32){leaf(r);} fn run()->i32{let mut x=3;forward(&mut x);x}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "forward_reference_parameter",
    "source": "fn leaf(r:&mut i32){*r=7;} fn forward(r:&mut i32){leaf(r);} fn run()->i32{let mut x=3;forward(&mut x);x}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn leaf(r:&mut i32){*r=*r+2;} fn outer(r:&mut i32){leaf(r);leaf(r);} fn run()->i32{let mut x=3;outer(&mut x);x}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "nested_mutator_calls",
    "source": "fn leaf(r:&mut i32){*r=*r+2;} fn outer(r:&mut i32){leaf(r);leaf(r);} fn run()->i32{let mut x=3;outer(&mut x);x}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn bump(r:&mut i32,n:i32){if n>0{*r=*r+1;bump(r,n-1);}} fn run()->i32{let mut x=3;bump(&mut x,4);x}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "recursive_reference_mutation",
    "source": "fn bump(r:&mut i32,n:i32){if n>0{*r=*r+1;bump(r,n-1);}} fn run()->i32{let mut x=3;bump(&mut x,4);x}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn leaf(r:&i32)->i32{*r} fn forward(r:&i32)->i32{leaf(r)} fn run()->i32{let x=7;forward(&x)}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "forward_shared_parameter",
    "source": "fn leaf(r:&i32)->i32{*r} fn forward(r:&i32)->i32{leaf(r)} fn run()->i32{let x=7;forward(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn run()->i32{let mut x=3;let r=&mut x;let q:&mut i32=r;*q=7;*r}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "typed_local_expected_reborrow",
    "source": "fn run()->i32{let mut x=3;let r=&mut x;let q:&mut i32=r;*q=7;*r}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn run()->i32{let mut x=3;let r=&mut x;let q:&mut i32;q=r;*q=7;*r}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "assignment_expected_reborrow",
    "source": "fn run()->i32{let mut x=3;let r=&mut x;let q:&mut i32;q=r;*q=7;*r}"
  },
  {
    "expected": 13,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn sum(a:&i32,b:i32)->i32{*a+b} fn run()->i32{let mut x=3;let r=&mut x;let n=sum(r,*r);*r=7;n+*r}\nfn main() { if run() == 13 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "mutable_to_shared_call_reborrow",
    "source": "fn sum(a:&i32,b:i32)->i32{*a+b} fn run()->i32{let mut x=3;let r=&mut x;let n=sum(r,*r);*r=7;n+*r}"
  },
  {
    "expected": 10,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn read(r:&i32)->i32{*r} fn run()->i32{let mut x=3;let n=read(&x);x=7;n+x}\nfn main() { if run() == 10 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "noescape_call_loan_ends",
    "source": "fn read(r:&i32)->i32{*r} fn run()->i32{let mut x=3;let n=read(&x);x=7;n+x}"
  },
  {
    "expected": 37,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn sum(r:&(i32,i32))->i32{(*r).0*10+(*r).1} fn run()->i32{let p=(3,7);sum(&p)}\nfn main() { if run() == 37 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "copy_aggregate_reference_parameter",
    "source": "fn sum(r:&(i32,i32))->i32{(*r).0*10+(*r).1} fn run()->i32{let p=(3,7);sum(&p)}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn read(r:&usize)->usize{*r} fn run()->i32{let x:usize=18446744073709551615usize;if read(&x)==18446744073709551615usize{7}else{0}}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "full64_usize_reference_parameter",
    "source": "fn read(r:&usize)->usize{*r} fn run()->i32{let x:usize=18446744073709551615usize;if read(&x)==18446744073709551615usize{7}else{0}}"
  },
  {
    "expected": "E0499",
    "full_source": "fn sum(a:&mut i32,b:&mut i32)->i32{*a+*b} fn run()->i32{let mut x=7;sum(&mut x,&mut x)}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "mutable_alias_arguments",
    "source": "fn sum(a:&mut i32,b:&mut i32)->i32{*a+*b} fn run()->i32{let mut x=7;sum(&mut x,&mut x)}"
  },
  {
    "expected": "E0502",
    "full_source": "fn sum(a:&i32,b:&mut i32)->i32{*a+*b} fn run()->i32{let mut x=7;sum(&x,&mut x)}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "shared_mutable_alias_arguments",
    "source": "fn sum(a:&i32,b:&mut i32)->i32{*a+*b} fn run()->i32{let mut x=7;sum(&x,&mut x)}"
  },
  {
    "expected": "E0594",
    "full_source": "fn write(r:&i32){*r=7;} fn run()->i32{let x=3;write(&x);x}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "shared_parameter_cannot_write",
    "source": "fn write(r:&i32){*r=7;} fn run()->i32{let x=3;write(&x);x}"
  },
  {
    "expected": "E0382",
    "full_source": "fn read(r:&mut i32)->i32{let q=r;*q+*r} fn run()->i32{let mut x=7;read(&mut x)}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "formal_handle_used_after_move",
    "source": "fn read(r:&mut i32)->i32{let q=r;*q+*r} fn run()->i32{let mut x=7;read(&mut x)}"
  },
  {
    "expected": "E0506",
    "full_source": "fn sum(a:&i32,b:i32)->i32{*a+b} fn run()->i32{let mut x=3;sum(&x,{x=7;0})}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "first_argument_loan_later_mutation",
    "source": "fn sum(a:&i32,b:i32)->i32{*a+b} fn run()->i32{let mut x=3;sum(&x,{x=7;0})}"
  },
  {
    "expected": "E0503",
    "full_source": "fn sum(a:&mut i32,b:i32)->i32{*a+b} fn run()->i32{let mut x=3;sum(&mut x,x)}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "first_mutable_argument_later_owner_read",
    "source": "fn sum(a:&mut i32,b:i32)->i32{*a+b} fn run()->i32{let mut x=3;sum(&mut x,x)}"
  },
  {
    "expected": "E0596",
    "full_source": "fn write(r:&mut i32){*r=7;} fn run()->i32{let x=3;write(&mut x);x}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "mutable_parameter_immutable_owner",
    "source": "fn write(r:&mut i32){*r=7;} fn run()->i32{let x=3;write(&mut x);x}"
  },
  {
    "expected": "E0499",
    "full_source": "fn sum(a:&mut i32,b:&mut i32)->i32{*a+*b} fn run()->i32{let mut a=[3,7];sum(&mut a[0],&mut a[1])}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "mutable_array_index_arguments_overlap",
    "source": "fn sum(a:&mut i32,b:&mut i32)->i32{*a+*b} fn run()->i32{let mut a=[3,7];sum(&mut a[0],&mut a[1])}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn pass(r:&i32)->&i32{r} fn run()->i32{let x=7;*pass(&x)}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "positive",
    "name": "reference_return_escape",
    "source": "fn pass(r:&i32)->&i32{r} fn run()->i32{let x=7;*pass(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "struct R<'a>{r:&'a i32} fn read(r:&i32)->i32{*r} fn run()->i32{let x=7;let r=R{r:&x};read(r.r)}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "reference_aggregate_field",
    "source": "struct R<'a>{r:&'a i32} fn read(r:&i32)->i32{*r} fn run()->i32{let x=7;let r=R{r:&x};read(r.r)}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn read(r:&&i32)->i32{**r} fn run()->i32{let x=7;let r=&x;read(&r)}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "reference_to_reference_parameter",
    "source": "fn read(r:&&i32)->i32{**r} fn run()->i32{let x=7;let r=&x;read(&r)}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow params oracle ok\n",
    "full_source": "fn read(r:&i32)->i32{*r} fn run()->i32{read(&7)}\nfn main() { if run() == 7 { println!(\"borrow params oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "rvalue_borrow_argument",
    "source": "fn read(r:&i32)->i32{*r} fn run()->i32{read(&7)}"
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
            prefix="jr-reference-calls-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "borrowing-b2-reference-calls-v1",
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
            raise SystemExit(f"{totals['failure']} B2 reference-call discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
