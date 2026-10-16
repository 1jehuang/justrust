#!/usr/bin/env python3
"""Bounded B1 borrowing differential corpus, not full Rust compatibility.

Exact frozen stock-classified sources/stdout mirror tests/borrowing.rs. Only the
approved B1 reference/loan/dereference slice is claimed. Legal unsupported syntax
is kept as explicit boundaries, not invalid Rust or parity. All valid executions
must exit zero with exact stdout and empty stderr. No trap exception applies.
Reuse module_correctness Case/run_case/process deadlines, bounded captures and
supplied-native workers 1/8 check/build. Never build a native binary here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "fa72f45eb692933c1db431a4a6e25c795f5350472c94db9c1e80a549e0e73c40"

# Original immutable acceptance artifact hash retained above.
# E1 native-pass promotion: reference_parameter_result is now positive; source/stdout/value unchanged.
# Authoritative bear source bytes. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "expected": 7,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let x=7;let r:&i32=&x;*r}\nfn main() { if run() == 7 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "shared_dereference",
    "source": "fn run()->i32{let x=7;let r:&i32=&x;*r}"
  },
  {
    "expected": 14,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let x=7;let r=&x;let s=r;*r+*s}\nfn main() { if run() == 14 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "shared_reference_copy",
    "source": "fn run()->i32{let x=7;let r=&x;let s=r;*r+*s}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let mut x=3;let r=&mut x;*r=*r+4;*r}\nfn main() { if run() == 7 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "immutable_carrier_mutable_referent",
    "source": "fn run()->i32{let mut x=3;let r=&mut x;*r=*r+4;*r}"
  },
  {
    "expected": 10,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let mut x=3;let r=&x;let n=*r;x=7;n+x}\nfn main() { if run() == 10 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "shared_nll_last_use",
    "source": "fn run()->i32{let mut x=3;let r=&x;let n=*r;x=7;n+x}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let mut x=3;let r=&mut x;*r=7;x}\nfn main() { if run() == 7 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "mutable_nll_last_use",
    "source": "fn run()->i32{let mut x=3;let r=&mut x;*r=7;x}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let mut x=3;let r=&mut x;let s=r;*s=7;x}\nfn main() { if run() == 7 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "mutable_reference_move",
    "source": "fn run()->i32{let mut x=3;let r=&mut x;let s=r;*s=7;x}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let mut x=3;let r=&mut x;let s=&mut *r;*s=7;*r}\nfn main() { if run() == 7 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "explicit_mutable_reborrow",
    "source": "fn run()->i32{let mut x=3;let r=&mut x;let s=&mut *r;*s=7;*r}"
  },
  {
    "expected": 10,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let mut x=3;let r=&mut x;let s=&*r;let n=*s;*r=7;n+*r}\nfn main() { if run() == 10 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "explicit_shared_reborrow",
    "source": "fn run()->i32{let mut x=3;let r=&mut x;let s=&*r;let n=*s;*r=7;n+*r}"
  },
  {
    "expected": 11,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "struct S{a:i32,b:i32} fn run()->i32{let mut p=S{a:3,b:7};let a=&mut p.a;let b=&mut p.b;*a=*a+1;*a+*b}\nfn main() { if run() == 11 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "disjoint_struct_fields",
    "source": "struct S{a:i32,b:i32} fn run()->i32{let mut p=S{a:3,b:7};let a=&mut p.a;let b=&mut p.b;*a=*a+1;*a+*b}"
  },
  {
    "expected": 10,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn choose(b:bool)->i32{let x=3;let y=7;let r:&i32;if b{r=&x;}else{r=&y;}*r} fn run()->i32{choose(true)+choose(false)}\nfn main() { if run() == 10 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "branch_reference_origins",
    "source": "fn choose(b:bool)->i32{let x=3;let y=7;let r:&i32;if b{r=&x;}else{r=&y;}*r} fn run()->i32{choose(true)+choose(false)}"
  },
  {
    "expected": 3,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let mut x=0;let mut i=0;while i<3{let r=&mut x;*r=*r+1;i=i+1;}x}\nfn main() { if run() == 3 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "loop_last_use",
    "source": "fn run()->i32{let mut x=0;let mut i=0;while i<3{let r=&mut x;*r=*r+1;i=i+1;}x}"
  },
  {
    "expected": 10,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let a=[3,7];let r=&a[0];let s=&a[1];*r+*s}\nfn main() { if run() == 10 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "shared_array_elements",
    "source": "fn run()->i32{let a=[3,7];let r=&a[0];let s=&a[1];*r+*s}"
  },
  {
    "expected": 37,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let p=(3,7);let r=&p;let q=*r;q.0*10+q.1}\nfn main() { if run() == 37 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "copy_aggregate_dereference",
    "source": "fn run()->i32{let p=(3,7);let r=&p;let q=*r;q.0*10+q.1}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let mut x=3;{let r=&mut x;*r=7;}x}\nfn main() { if run() == 7 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "lexical_mutable_loan_scope",
    "source": "fn run()->i32{let mut x=3;{let r=&mut x;*r=7;}x}"
  },
  {
    "expected": "E0594",
    "full_source": "fn run()->i32{let mut x=3;let mut r=&x;*r=7;*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "mutable_carrier_shared_cannot_write",
    "source": "fn run()->i32{let mut x=3;let mut r=&x;*r=7;*r}"
  },
  {
    "expected": "E0596",
    "full_source": "fn run()->i32{let x=3;let r=&mut x;*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "mutable_borrow_immutable_owner",
    "source": "fn run()->i32{let x=3;let r=&mut x;*r}"
  },
  {
    "expected": "E0502",
    "full_source": "fn run()->i32{let mut x=3;let r=&x;let s=&mut x;*r+*s}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "shared_mutable_conflict",
    "source": "fn run()->i32{let mut x=3;let r=&x;let s=&mut x;*r+*s}"
  },
  {
    "expected": "E0499",
    "full_source": "fn run()->i32{let mut x=3;let r=&mut x;let s=&mut x;*r+*s}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "two_mutable_loans",
    "source": "fn run()->i32{let mut x=3;let r=&mut x;let s=&mut x;*r+*s}"
  },
  {
    "expected": "E0506",
    "full_source": "fn run()->i32{let mut x=3;let r=&x;x=7;*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "owner_write_while_loan_live",
    "source": "fn run()->i32{let mut x=3;let r=&x;x=7;*r}"
  },
  {
    "expected": "E0505",
    "full_source": "struct S{n:i32} fn run()->i32{let x=S{n:7};let r=&x;let y=x;(*r).n+y.n}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "owner_move_while_loan_live",
    "source": "struct S{n:i32} fn run()->i32{let x=S{n:7};let r=&x;let y=x;(*r).n+y.n}"
  },
  {
    "expected": "E0507",
    "full_source": "struct S{n:i32} fn run()->i32{let x=S{n:7};let r=&x;let y=*r;y.n}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "noncopy_move_through_shared",
    "source": "struct S{n:i32} fn run()->i32{let x=S{n:7};let r=&x;let y=*r;y.n}"
  },
  {
    "expected": "E0382",
    "full_source": "fn run()->i32{let mut x=3;let r=&mut x;let s=r;*s=7;*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "mutable_reference_moved_carrier",
    "source": "fn run()->i32{let mut x=3;let r=&mut x;let s=r;*s=7;*r}"
  },
  {
    "expected": "E0499",
    "full_source": "fn run()->i32{let mut a=[3,7];let r=&mut a[0];let s=&mut a[1];*r+*s}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "array_index_loans_overlap",
    "source": "fn run()->i32{let mut a=[3,7];let r=&mut a[0];let s=&mut a[1];*r+*s}"
  },
  {
    "expected": "E0597",
    "full_source": "fn run()->i32{let r:&i32;{let x=7;r=&x;}*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "referent_storage_dead",
    "source": "fn run()->i32{let r:&i32;{let x=7;r=&x;}*r}"
  },
  {
    "expected": "E0506",
    "full_source": "fn run()->i32{let mut x=3;let r=&mut x;let s=&mut *r;*r=7;*s}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "parent_access_while_child_live",
    "source": "fn run()->i32{let mut x=3;let r=&mut x;let s=&mut *r;*r=7;*s}"
  },
  {
    "expected": "E0597",
    "full_source": "fn run()->i32{let x=0;let mut r:&i32=&x;let mut i=0;while i<2{let y=i;r=&y;i=i+1;}*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "loop_referent_storage_dead",
    "source": "fn run()->i32{let x=0;let mut r:&i32=&x;let mut i=0;while i<2{let y=i;r=&y;i=i+1;}*r}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn pass(r:&i32)->&i32{r} fn run()->i32{let x=7;*pass(&x)}\nfn main() { if run() == 7 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "reference_parameter_result",
    "source": "fn pass(r:&i32)->&i32{r} fn run()->i32{let x=7;*pass(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "struct R<'a>{r:&'a i32} fn run()->i32{let x=7;let r=R{r:&x};*r.r}\nfn main() { if run() == 7 { println!(\"borrow oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "reference_aggregate_field",
    "source": "struct R<'a>{r:&'a i32} fn run()->i32{let x=7;let r=R{r:&x};*r.r}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let x=7;let r=&x;let rr=&r;**rr}\nfn main() { if run() == 7 { println!(\"borrow oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "reference_to_reference",
    "source": "fn run()->i32{let x=7;let r=&x;let rr=&r;**rr}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let r=&7;*r}\nfn main() { if run() == 7 { println!(\"borrow oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "rvalue_borrow",
    "source": "fn run()->i32{let r=&7;*r}"
  },
  {
    "expected": 7,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let mut x=3;let s:&mut i32;{let r=&mut x;s=&mut *r;}*s=7;x}\nfn main() { if run() == 7 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "parent_handle_dies_child_lives",
    "source": "fn run()->i32{let mut x=3;let s:&mut i32;{let r=&mut x;s=&mut *r;}*s=7;x}"
  },
  {
    "expected": 10,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let x=3;let y=7;let mut r=&x;let n=*r;r=&y;n+*r}\nfn main() { if run() == 10 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "shared_carrier_reassignment",
    "source": "fn run()->i32{let x=3;let y=7;let mut r=&x;let n=*r;r=&y;n+*r}"
  },
  {
    "expected": "E0506",
    "full_source": "fn run()->i32{let mut x=3;let y=7;let mut r=&x;let s=r;r=&y;x=9;*s+*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "shared_copy_survives_carrier_overwrite",
    "source": "fn run()->i32{let mut x=3;let y=7;let mut r=&x;let s=r;r=&y;x=9;*s+*r}"
  },
  {
    "expected": "E0506",
    "full_source": "fn run()->i32{let mut x=3;let r=&mut x;let s=&*r;*r=7;*s}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "shared_child_freezes_parent_write",
    "source": "fn run()->i32{let mut x=3;let r=&mut x;let s=&*r;*r=7;*s}"
  },
  {
    "expected": 4,
    "expected_stdout": "borrow oracle ok\n",
    "full_source": "fn run()->i32{let mut x=1;let mut y=2;let mut r=&mut x;*r={r=&mut y;3};let w:usize=4294967296;let p=&w;if *p==4294967296{x+y}else{0}}\nfn main() { if run() == 4 { println!(\"borrow oracle ok\"); } }\n",
    "group": "positive",
    "name": "deref_lhs_resolved_after_rhs",
    "source": "fn run()->i32{let mut x=1;let mut y=2;let mut r=&mut x;*r={r=&mut y;3};let w:usize=4294967296;let p=&w;if *p==4294967296{x+y}else{0}}"
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
    assert len(cases) == 35 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-borrowing-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "borrowing-b1-v1",
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
            raise SystemExit(f"{totals['failure']} borrowing-B1 discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
