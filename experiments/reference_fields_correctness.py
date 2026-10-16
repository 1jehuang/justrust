#!/usr/bin/env python3
"""Bounded E2 reference-fields differential corpus, not full Rust compatibility.

Exact frozen stock-classified source bytes/stdout mirror bear's E2 acceptance
corpus. Builtin single-dereference fields and the independently proved unchanged
user_method_autoderef source are supported. General computed assignment and
runtime reference indexing remain explicit boundaries. Valid runs must exit zero, exact stdout, empty stderr.
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


ACCEPTANCE_CORPUS_SHA256 = "01a34702d4e36424cc5d0ce35d900ee4ad00a2b6af174b3589fedeb2877b4f8b"

# Authoritative frozen bear oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "group": "positive",
    "name": "shared_struct_field_read",
    "source": "struct S{x:i32} fn run()->i32{let s=S{x:7};let r=&s;r.x}",
    "expected": 7,
    "full_source": "struct S{x:i32} fn run()->i32{let s=S{x:7};let r=&s;r.x}\nfn main() { if run() == 7 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "positive",
    "name": "shared_tuple_field_read",
    "source": "fn run()->i32{let p=(3,7);let r=&p;r.0*10+r.1}",
    "expected": 37,
    "full_source": "fn run()->i32{let p=(3,7);let r=&p;r.0*10+r.1}\nfn main() { if run() == 37 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutable_struct_tuple_writes",
    "source": "struct S{x:i32} fn run()->i32{let mut s=S{x:3};let mut p=(3,4);let r=&mut s;let q=&mut p;r.x=7;q.1=8;s.x+p.1}",
    "expected": 15,
    "full_source": "struct S{x:i32} fn run()->i32{let mut s=S{x:3};let mut p=(3,4);let r=&mut s;let q=&mut p;r.x=7;q.1=8;s.x+p.1}\nfn main() { if run() == 15 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "positive",
    "name": "shared_field_borrow_return",
    "source": "struct S{x:i32} fn get(r:&S)->&i32{&r.x} fn run()->i32{let s=S{x:7};*get(&s)}",
    "expected": 7,
    "full_source": "struct S{x:i32} fn get(r:&S)->&i32{&r.x} fn run()->i32{let s=S{x:7};*get(&s)}\nfn main() { if run() == 7 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutable_field_borrow_return",
    "source": "struct S{x:i32} fn get(r:&mut S)->&mut i32{&mut r.x} fn run()->i32{let mut s=S{x:3};let q=get(&mut s);*q=7;s.x}",
    "expected": 7,
    "full_source": "struct S{x:i32} fn get(r:&mut S)->&mut i32{&mut r.x} fn run()->i32{let mut s=S{x:3};let q=get(&mut s);*q=7;s.x}\nfn main() { if run() == 7 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "positive",
    "name": "nested_fields_static_array",
    "source": "struct Inner{p:[i32;2]} struct Outer{inner:Inner} fn run()->i32{let mut s=Outer{inner:Inner{p:[3,4]}};let r=&mut s;r.inner.p[1]=7;r.inner.p[1]}",
    "expected": 7,
    "full_source": "struct Inner{p:[i32;2]} struct Outer{inner:Inner} fn run()->i32{let mut s=Outer{inner:Inner{p:[3,4]}};let r=&mut s;r.inner.p[1]=7;r.inner.p[1]}\nfn main() { if run() == 7 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "positive",
    "name": "genuine_reference_call_field_reads",
    "source": "struct S{x:i32} fn shared(s:&S)->&S{s} fn unique(s:&mut S)->&mut S{s} fn run()->i32{let s=S{x:3};let mut t=S{x:7};shared(&s).x*10+unique(&mut t).x}",
    "expected": 37,
    "full_source": "struct S{x:i32} fn shared(s:&S)->&S{s} fn unique(s:&mut S)->&mut S{s} fn run()->i32{let s=S{x:3};let mut t=S{x:7};shared(&s).x*10+unique(&mut t).x}\nfn main() { if run() == 37 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "positive",
    "name": "explicit_deref_parity",
    "source": "struct S{x:i32} fn run()->i32{let mut s=S{x:3};let r=&mut s;r.x=7;if r.x==(*r).x{r.x}else{0}}",
    "expected": 7,
    "full_source": "struct S{x:i32} fn run()->i32{let mut s=S{x:3};let r=&mut s;r.x=7;if r.x==(*r).x{r.x}else{0}}\nfn main() { if run() == 7 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "positive",
    "name": "disjoint_reference_field_borrows",
    "source": "struct S{x:i32,y:i32} fn run()->i32{let mut s=S{x:3,y:4};let r=&mut s;let a=&mut r.x;let b=&mut r.y;*a=7;*b=8;r.x+r.y}",
    "expected": 15,
    "full_source": "struct S{x:i32,y:i32} fn run()->i32{let mut s=S{x:3,y:4};let r=&mut s;let a=&mut r.x;let b=&mut r.y;*a=7;*b=8;r.x+r.y}\nfn main() { if run() == 15 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "positive",
    "name": "field_loan_lastuse_resume",
    "source": "struct S{x:i32} fn run()->i32{let mut s=S{x:3};let r=&mut s;let q=&r.x;let n=*q;r.x=7;n+r.x}",
    "expected": 10,
    "full_source": "struct S{x:i32} fn run()->i32{let mut s=S{x:3};let r=&mut s;let q=&r.x;let n=*q;r.x=7;n+r.x}\nfn main() { if run() == 10 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "negative",
    "name": "noncopy_field_move",
    "source": "struct N{x:i32} struct S{n:N} fn run()->i32{let s=S{n:N{x:7}};let r=&s;let n=r.n;n.x}",
    "expected": "E0507",
    "full_source": "struct N{x:i32} struct S{n:N} fn run()->i32{let s=S{n:N{x:7}};let r=&s;let n=r.n;n.x}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "shared_reference_field_write",
    "source": "struct S{x:i32} fn run()->i32{let s=S{x:3};let r=&s;r.x=7;r.x}",
    "expected": "E0594",
    "full_source": "struct S{x:i32} fn run()->i32{let s=S{x:3};let r=&s;r.x=7;r.x}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "private_reference_field",
    "source": "mod m{pub struct S{x:i32} pub fn make()->S{S{x:7}}} fn run()->i32{let s=m::make();let r=&s;r.x}",
    "expected": "E0616",
    "full_source": "mod m{pub struct S{x:i32} pub fn make()->S{S{x:7}}} fn run()->i32{let s=m::make();let r=&s;r.x}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "local_field_return",
    "source": "struct S{x:i32} fn bad(r:&S)->&i32{let s=S{x:7};&s.x} fn run()->i32{7}",
    "expected": "E0515",
    "full_source": "struct S{x:i32} fn bad(r:&S)->&i32{let s=S{x:7};&s.x} fn run()->i32{7}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "field_result_owner_storage_dead",
    "source": "struct S{x:i32} fn get(r:&S)->&i32{&r.x} fn run()->i32{let q:&i32;{let s=S{x:7};q=get(&s);}*q}",
    "expected": "E0597",
    "full_source": "struct S{x:i32} fn get(r:&S)->&i32{&r.x} fn run()->i32{let q:&i32;{let s=S{x:7};q=get(&s);}*q}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "field_write_while_child_live",
    "source": "struct S{x:i32} fn run()->i32{let mut s=S{x:3};let r=&mut s;let q=&mut r.x;r.x=7;*q}",
    "expected": "E0506",
    "full_source": "struct S{x:i32} fn run()->i32{let mut s=S{x:3};let r=&mut s;let q=&mut r.x;r.x=7;*q}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "legalunsupported",
    "name": "computed_call_field_assignment",
    "source": "struct S{x:i32} fn get(r:&mut S)->&mut S{r} fn run()->i32{let mut s=S{x:3};get(&mut s).x=7;s.x}",
    "expected": 7,
    "full_source": "struct S{x:i32} fn get(r:&mut S)->&mut S{r} fn run()->i32{let mut s=S{x:3};get(&mut s).x=7;s.x}\nfn main() { if run() == 7 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "borrow_call_field_rvalue",
    "source": "struct S{x:i32} fn get(r:&S)->&S{r} fn run()->i32{let s=S{x:7};let q=&get(&s).x;*q}",
    "expected": 7,
    "full_source": "struct S{x:i32} fn get(r:&S)->&S{r} fn run()->i32{let s=S{x:7};let q=&get(&s).x;*q}\nfn main() { if run() == 7 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "bare_reference_index",
    "source": "fn run()->i32{let a=[3,7];let r=&a;r[1]}",
    "expected": 7,
    "full_source": "fn run()->i32{let a=[3,7];let r=&a;r[1]}\nfn main() { if run() == 7 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
  },
  {
    "group": "positive",
    "name": "user_method_autoderef",
    "source": "struct S{x:i32} impl S{fn value(&self)->i32{self.x}} fn run()->i32{let s=S{x:7};let r=&s;r.value()}",
    "expected": 7,
    "full_source": "struct S{x:i32} impl S{fn value(&self)->i32{self.x}} fn run()->i32{let s=S{x:7};let r=&s;r.value()}\nfn main() { if run() == 7 { println!(\"reference fields oracle ok\"); } }\n",
    "expected_stdout": "reference fields oracle ok\n"
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
            prefix="jr-reference-fields-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "borrowing-e2-reference-fields-v1",
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
            raise SystemExit(f"{totals['failure']} E2 reference-fields discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
