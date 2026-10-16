#!/usr/bin/env python3
"""Bounded B3a shared-reference-return differential corpus, not full Rust compatibility.

Exact frozen stock-classified source bytes/stdout mirror bear's B3a acceptance
corpus. Only explicit-region whole shared-parameter reference returns are
claimed. Mutable/projected/aggregate returns and broader elision
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


ACCEPTANCE_CORPUS_SHA256 = "87a4940bb3b2800a6d285102ede7658ef1ed019149f7e73d17fffdbf6fad9b9a"

# Original immutable acceptance artifact hash retained above.
# E1 native-pass promotion: elided_result is now positive; source/stdout/value unchanged.
# Authoritative bear source bytes. full_source has no added wrapper.
# E2 native-pass promotion: projected_result is positive; source/stdout/value unchanged.
# Original immutable acceptance artifact hash above is retained.
# E3 classification promotion: mutable_input_shared_result; exact source/stdout/value retained.
# Original immutable acceptance artifact hash above is unchanged.
FIXTURES = json.loads(r'''
[
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let x=7;*id(&x)}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "identity",
    "source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let x=7;*id(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn id<'renamed>(x:&'renamed i32)->&'renamed i32{x} fn run()->i32{let x=7;*id(&x)}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "alpha_renamed",
    "source": "fn id<'renamed>(x:&'renamed i32)->&'renamed i32{x} fn run()->i32{let x=7;*id(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn choose<'a>(x:&'a i32,y:&'a i32,b:bool)->&'a i32{if b{x}else{y}} fn run()->i32{let x=3;let y=7;*choose(&x,&y,false)}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "same_region_select",
    "source": "fn choose<'a>(x:&'a i32,y:&'a i32,b:bool)->&'a i32{if b{x}else{y}} fn run()->i32{let x=3;let y=7;*choose(&x,&y,false)}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn choose<'a,'b>(x:&'a i32,y:&'b i32)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=3;r=choose(&x,&y);}*r}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "independent_short_input",
    "source": "fn choose<'a,'b>(x:&'a i32,y:&'b i32)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=3;r=choose(&x,&y);}*r}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn choose<'a,'b>(x:&'a i32,y:&'b bool)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=true;r=choose(&x,&y);}*r}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "independent_different_pointee",
    "source": "fn choose<'a,'b>(x:&'a i32,y:&'b bool)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=true;r=choose(&x,&y);}*r}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn short<'a:'b,'b>(x:&'a i32)->&'b i32{x} fn run()->i32{let x=7;*short(&x)}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "outlives_shortening",
    "source": "fn short<'a:'b,'b>(x:&'a i32)->&'b i32{x} fn run()->i32{let x=7;*short(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn forward<'a>(x:&'a i32)->&'a i32{id(x)} fn run()->i32{let x=7;*forward(&x)}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "forward_return",
    "source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn forward<'a>(x:&'a i32)->&'a i32{id(x)} fn run()->i32{let x=7;*forward(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn recur<'a>(x:&'a i32,n:i32)->&'a i32{if n==0{x}else{recur(x,n-1)}} fn run()->i32{let x=7;*recur(&x,3)}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "recursive_return",
    "source": "fn recur<'a>(x:&'a i32,n:i32)->&'a i32{if n==0{x}else{recur(x,n-1)}} fn run()->i32{let x=7;*recur(&x,3)}"
  },
  {
    "expected": 10,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let mut x=3;let r=id(&x);let n=*r;x=7;n+x}\nfn main() { if run() == 10 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "last_use_then_mutation",
    "source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let mut x=3;let r=id(&x);let n=*r;x=7;n+x}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let a=&x;r=id(a);}*r}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "argument_carrier_storage_dead",
    "source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let a=&x;r=id(a);}*r}"
  },
  {
    "expected": 14,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let x=7;let r=id(&x);let s=r;*r+*s}\nfn main() { if run() == 14 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "returned_shared_copy",
    "source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let x=7;let r=id(&x);let s=r;*r+*s}"
  },
  {
    "expected": 37,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn id<'a>(x:&'a(i32,i32))->&'a(i32,i32){x} fn run()->i32{let x=(3,7);let r=id(&x);(*r).0*10+(*r).1}\nfn main() { if run() == 37 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "whole_tuple_result",
    "source": "fn id<'a>(x:&'a(i32,i32))->&'a(i32,i32){x} fn run()->i32{let x=(3,7);let r=id(&x);(*r).0*10+(*r).1}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "struct S{x:i32} fn id<'a>(x:&'a S)->&'a S{x} fn run()->i32{let x=S{x:7};(*id(&x)).x}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "whole_struct_result",
    "source": "struct S{x:i32} fn id<'a>(x:&'a S)->&'a S{x} fn run()->i32{let x=S{x:7};(*id(&x)).x}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn id<'a>(x:&'a usize)->&'a usize{x} fn run()->i32{let x=18446744073709551615usize;if *id(&x)==18446744073709551615usize{7}else{0}}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "full64_result_transport",
    "source": "fn id<'a>(x:&'a usize)->&'a usize{x} fn run()->i32{let x=18446744073709551615usize;if *id(&x)==18446744073709551615usize{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn id<'a>(x:&'a i32)->&'a i32{let r=&*x;r} fn run()->i32{let x=7;*id(&x)}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "private_explicit_reborrow",
    "source": "fn id<'a>(x:&'a i32)->&'a i32{let r=&*x;r} fn run()->i32{let x=7;*id(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn leaf<'a,'b>(x:&'a i32,y:&'b bool)->&'a i32{x} fn forward<'a,'b>(x:&'a i32,y:&'b bool)->&'a i32{leaf(x,y)} fn run()->i32{let x=7;let r:&i32;{let y=true;r=forward(&x,&y);}*r}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "forward_independent_dependency",
    "source": "fn leaf<'a,'b>(x:&'a i32,y:&'b bool)->&'a i32{x} fn forward<'a,'b>(x:&'a i32,y:&'b bool)->&'a i32{leaf(x,y)} fn run()->i32{let x=7;let r:&i32;{let y=true;r=forward(&x,&y);}*r}"
  },
  {
    "expected": 15,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn choose<'a>(x:&'a i32,y:&'a i32,b:bool)->&'a i32{if b{x}else{y}} fn run()->i32{let mut x=3;let mut y=7;let r=choose(&x,&y,true);let n=*r;x=4;y=8;n+x+y}\nfn main() { if run() == 15 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "branch_alias_last_use",
    "source": "fn choose<'a>(x:&'a i32,y:&'a i32,b:bool)->&'a i32{if b{x}else{y}} fn run()->i32{let mut x=3;let mut y=7;let r=choose(&x,&y,true);let n=*r;x=4;y=8;n+x+y}"
  },
  {
    "expected": "E0515",
    "expected_stdout": "",
    "full_source": "fn bad<'a>(x:&'a i32)->&'a i32{let y=7;&y} fn run()->i32{let x=3;*bad(&x)}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "callee_owned_return",
    "source": "fn bad<'a>(x:&'a i32)->&'a i32{let y=7;&y} fn run()->i32{let x=3;*bad(&x)}"
  },
  {
    "expected": "E0515",
    "expected_stdout": "",
    "full_source": "fn bad<'a>()->&'a i32{let y=7;&y} fn run()->i32{7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "unused_bad_callee",
    "source": "fn bad<'a>()->&'a i32{let y=7;&y} fn run()->i32{7}"
  },
  {
    "expected": null,
    "expected_stdout": "",
    "full_source": "fn bad<'a,'b>(x:&'a i32,y:&'b i32)->&'a i32{y} fn run()->i32{7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "unrelated_input_lifetime",
    "source": "fn bad<'a,'b>(x:&'a i32,y:&'b i32)->&'a i32{y} fn run()->i32{7}"
  },
  {
    "expected": "E0597",
    "expected_stdout": "",
    "full_source": "fn choose<'a>(x:&'a i32,y:&'a i32)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=3;r=choose(&x,&y);}*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "same_region_short_owner",
    "source": "fn choose<'a>(x:&'a i32,y:&'a i32)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=3;r=choose(&x,&y);}*r}"
  },
  {
    "expected": "E0597",
    "expected_stdout": "",
    "full_source": "fn choose<'a>(x:&'a i32,y:&'a bool)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=true;r=choose(&x,&y);}*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "different_pointee_same_region",
    "source": "fn choose<'a>(x:&'a i32,y:&'a bool)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=true;r=choose(&x,&y);}*r}"
  },
  {
    "expected": "E0597",
    "expected_stdout": "",
    "full_source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let r:&i32;{let x=7;r=id(&x);}*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "owner_dies_before_result",
    "source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let r:&i32;{let x=7;r=id(&x);}*r}"
  },
  {
    "expected": "E0506",
    "expected_stdout": "",
    "full_source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let mut x=3;let r=id(&x);x=7;*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "owner_mutation_before_result_use",
    "source": "fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let mut x=3;let r=id(&x);x=7;*r}"
  },
  {
    "expected": "E0506",
    "expected_stdout": "",
    "full_source": "fn choose<'a>(x:&'a i32,y:&'a i32,b:bool)->&'a i32{if b{x}else{y}} fn run()->i32{let mut x=3;let y=7;let r=choose(&x,&y,false);x=9;*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "branch_candidate_mutation",
    "source": "fn choose<'a>(x:&'a i32,y:&'a i32,b:bool)->&'a i32{if b{x}else{y}} fn run()->i32{let mut x=3;let y=7;let r=choose(&x,&y,false);x=9;*r}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn id<'a>(x:&'a mut i32)->&'a mut i32{x} fn run()->i32{let mut x=3;*id(&mut x)=7;x}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "mutable_result",
    "source": "fn id<'a>(x:&'a mut i32)->&'a mut i32{x} fn run()->i32{let mut x=3;*id(&mut x)=7;x}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "struct S{x:i32} fn get<'a>(x:&'a S)->&'a i32{&x.x} fn run()->i32{let x=S{x:7};*get(&x)}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "projected_result",
    "source": "struct S{x:i32} fn get<'a>(x:&'a S)->&'a i32{&x.x} fn run()->i32{let x=S{x:7};*get(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn id(x:&i32)->&i32{x} fn run()->i32{let x=7;*id(&x)}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "elided_result",
    "source": "fn id(x:&i32)->&i32{x} fn run()->i32{let x=7;*id(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "static X:i32=7;fn get()->&'static i32{&X} fn run()->i32{*get()}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "static_result",
    "source": "static X:i32=7;fn get()->&'static i32{&X} fn run()->i32{*get()}"
  },
  {
    "expected": 7,
    "expected_stdout": "reference returns oracle ok\n",
    "full_source": "fn id<'a>(x:&'a mut i32)->&'a i32{&*x} fn run()->i32{let mut x=7;*id(&mut x)}\nfn main() { if run() == 7 { println!(\"reference returns oracle ok\"); } }\n",
    "group": "positive",
    "name": "mutable_input_shared_result",
    "source": "fn id<'a>(x:&'a mut i32)->&'a i32{&*x} fn run()->i32{let mut x=7;*id(&mut x)}"
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
            prefix="jr-reference-returns-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "borrowing-b3a-reference-returns-v1",
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
            raise SystemExit(f"{totals['failure']} B3a reference-return discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
