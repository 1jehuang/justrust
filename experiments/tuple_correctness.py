#!/usr/bin/env python3
"""Independent bounded tuple differential corpus, not a compatibility/speed claim.

Reuse the file-backed, deadline-bound module harness. Every fixture is first
classified by exact stock acceptance and expected runtime output. Legal features
outside this tuple slice are boundaries, never invalid Rust or parity. Native
runs cover both check/build and workers 1/8. --stock-only validates the oracle
corpus before an experimental binary is available. No generated compiler code
or compiler delegation is part of the compiler under test.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


def corpus():
    valid = [
        ("tuple_destructuring", "fn main(){let (n,b)=(3,true);if n==3 && b {println!(\"ok\");}}"),
        ("nested_literal", "fn main(){let t=(4,(true,9));if t.0+t.1.1==13 && t.1.0 {println!(\"ok\");}}"),
        ("annotated_nested_unit", "fn main(){let t:(i32,(bool,()),i32)=(3,(true,()),8);t.1.1;if t.0+t.2==11 && t.1.0 {println!(\"ok\");}}"),
        ("single_tuple_and_parentheses", "fn main(){let t:(i32,)=(7,);let x=(t.0);if x==7 {println!(\"ok\");}}"),
        ("temporary_projection", "fn f()->((i32,bool),i32){((5,true),2)} fn main(){if f().0.0+((1,3),4).0.1==8 && f().0.1 {println!(\"ok\");}}"),
        ("nested_mutable_projection", "fn main(){let mut t=(1,(false,3));t.1.0=true;t.1.1=8;t.0=4;if t.1.0 && t.0+t.1.1==12 {println!(\"ok\");}}"),
        ("whole_tuple_swap", "fn main(){let mut t=(2,9);t=(t.1,t.0);t=t;if t.0==9 && t.1==2 {println!(\"ok\");}}"),
        ("field_rhs_mutates_other_field", "fn main(){let mut t=(1,2);t.0={t.1=7;t.1+1};if t.0==8 && t.1==7 {println!(\"ok\");}}"),
        ("argument_snapshot", "fn f(a:(i32,i32),b:(i32,i32))->i32{a.0*100+a.1*10+b.0+b.1} fn main(){let mut t=(2,3);let n=f(t,{t.0=8;t.1=9;t});if n==247 && t.0==8 {println!(\"ok\");}}"),
        ("projected_argument_snapshot", "fn f(a:(i32,bool),b:(i32,bool))->bool{a.0==3 && a.1 && b.0==9 && !b.1} fn main(){let mut t=((3,true),0);if f(t.0,{t.0=(9,false);t.0}) {println!(\"ok\");}}"),
        ("construction_order", "fn main(){let mut n=0;let t=({n=n+1;n},{n=n*10;n}, {n=n+3;n});if t.0==1 && t.1==10 && t.2==13 && n==13 {println!(\"ok\");}}"),
        ("tuple_parameter_private_copy", "fn f(p:(i32,bool))->(i32,bool){let mut q=p;q.0=q.0+4;q.1=!q.1;q} fn main(){let t=(3,true);let r=f(t);if t.0==3 && t.1 && r.0==7 && !r.1 {println!(\"ok\");}}"),
        ("return_buffer_survives_later_call", "fn f(n:i32)->((i32,i32),(bool,i32)){((n,n+1),(true,n+2))} fn main(){let a=f(3);let b=f(20);if a.0.0==3 && a.0.1==4 && a.1.1==5 && b.0.0==20 {println!(\"ok\");}}"),
        ("recursive_nested_return", "fn f(n:i32)->((i32,bool),((),i32)){if n==0 {((2,true),((),3))}else{let p=f(n-1);((p.0.0+1,!p.0.1),((),p.1.1+2))}} fn main(){let r=f(6);if r.0.0==8 && r.0.1 && r.1.1==15 {println!(\"ok\");}}"),
        ("mutual_tuple_return", "fn a(n:i32)->(i32,(bool,i32)){if n==0 {(1,(true,2))}else{let p=b(n-1);(p.0+1,(p.1.0,p.1.1+3))}} fn b(n:i32)->(i32,(bool,i32)){a(n)} fn main(){let r=b(5);if r.0==6 && r.1.0 && r.1.1==17 {println!(\"ok\");}}"),
        ("if_join_tuple", "fn f(b:bool)->(i32,(bool,i32)){if b {(2,(true,7))}else{(8,(false,3))}} fn main(){let a=f(true);let b=f(false);if a.0+a.1.1==9 && a.1.0 && b.0+b.1.1==11 && !b.1.0 {println!(\"ok\");}}"),
        ("block_tuple_snapshot", "fn main(){let mut t=(1,2);let r={let old=t;t=(8,9);old};if r.0==1 && r.1==2 && t.0==8 {println!(\"ok\");}}"),
        ("loop_tuple_break", "fn main(){let mut n=0;let t=loop{n=n+1;if n==4 {break (n,(true,n+1));}};if t.0==4 && t.1.0 && t.1.1==5 {println!(\"ok\");}}"),
        ("early_return_in_constructor", "fn f()->(i32,bool){let t=(1,{return (7,true);false});(t.0,t.1)} fn main(){let t=f();if t.0==7 && t.1 {println!(\"ok\");}}"),
        ("early_return_in_argument", "fn g(a:i32,b:(i32,bool))->(i32,bool){(a+b.0,b.1)} fn f()->(i32,bool){g(1,{return (9,false);(3,true)})} fn main(){let t=f();if t.0==9 && !t.1 {println!(\"ok\");}}"),
        ("continue_during_constructor", "fn main(){let mut n=0;let t=loop{n=n+1;let p=(n,{if n<3 {continue;}n+1});break p;};if t.0==3 && t.1==4 {println!(\"ok\");}}"),
        ("nested_loop_tuple_result", "fn main(){let p=(10,loop{let inner=loop{break (2,true);};break (inner.0+3,inner.1);});if p.0+p.1.0==15 && p.1.1 {println!(\"ok\");}}"),
        ("zero_sized_tuple_return", "fn z(p:((),((),())))->((),((),())){p} fn main(){let mut n=0;let t=z(({n=n+1;},({n=n+1;},{n=n+1;})));t.0;t.1.0;t.1.1;if n==3 {println!(\"ok\");}}"),
        ("wide_tuple_no_register_limit", "fn f()->(i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32){(0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19)} fn main(){let t=f();if t.0+t.7+t.19==26 {println!(\"ok\");}}"),
        ("copied_shadow_and_module_signature", "mod a{pub fn f(p:(i32,bool))->(bool,i32){(p.1,p.0+1)}} use a::f as call; fn main(){let t=(4,true);let old=t;let t=call(t);if old.0==4 && old.1 && t.0 && t.1==5 {println!(\"ok\");}}"),
    ]
    for name, source in valid:
        yield Case(name, "valid", source, "ok\n")
    invalid = [
        ("annotation_shape", "fn main(){let t:(i32,bool)=(1,2,3);}"),
        ("annotation_nested_type", "fn main(){let t:(i32,(bool,i32))=(1,(4,2));}"),
        ("call_shape", "fn f(t:(i32,bool)){} fn main(){f((1,));}"),
        ("return_shape", "fn f()->(i32,bool){(1,2)} fn main(){}"),
        ("field_assignment_type", "fn main(){let mut t=(1,true);t.1=4;}"),
        ("projection_out_of_bounds", "fn main(){let t=(1,true);let x=t.2;}"),
        ("nested_projection_out_of_bounds", "fn main(){let t=((1,),true);let x=t.0.1;}"),
        ("immutable_field_assignment", "fn main(){let t=(1,true);t.0=2;}"),
        ("scalar_field_projection", "fn main(){let t=(1,true);let x=t.0.0;}"),
        ("tuple_if_branch_mismatch", "fn main(){let t=if true {(1,true)}else{(2,3)};}"),
    ]
    for name, source in invalid:
        yield Case(name, "invalid", source)
    for name, source in [
        ("tuple_equality", "fn main(){let t=(1,true);if t==(1,true){println!(\"ok\");}}"),
        ("tuple_ordering", "fn main(){if (1,2)<(2,1){println!(\"ok\");}}"),
        ("tuple_reference", "fn main(){let t=(1,true);let p=&t;if p.0==1 && p.1 {println!(\"ok\");}}"),
    ]:
        # E2 native-pass promotion only. Exact historical source/stdout retained.
        kind = "valid" if name == "tuple_reference" else "boundary"
        yield Case(name, kind, source, "ok\n")


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
    assert len(cases) == 38 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-tuple-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "tuples-v1",
                "case_count": len(cases), "stock_only": args.stock_only,
                "workers": [] if args.stock_only else [1, 8],
                "modes": [] if args.stock_only else ["build", "check"], "edition": 2024,
                "reference_modes": ["build", "run"], "rustc_path": str(rustc),
                "reference": version["stdout"],
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest() if binary else None,
                "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                "shared_harness_sha256": hashlib.sha256(Path(__file__).with_name("module_correctness.py").read_bytes()).hexdigest(),
                "corpus_sha256": corpus_hash,
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
            record(row)
            totals[row["outcome"]] += 1
            print(f"{case.name}: {row['outcome']}", flush=True)
        record({"record": "summary", **totals, "passed": totals["failure"] == 0})
        if totals["failure"]:
            raise SystemExit(f"{totals['failure']} tuple discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
