#!/usr/bin/env python3
"""Bounded semantic-type versus divergence differential corpus.

Uses the existing safe external validation runner. This is not part of native
compilation. Legal unsupported cases are classified separately from conformance.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, run_case


def cases():
    valid = [
        ("typed_dead_arithmetic", "fn f()->i32{10+{return 7;0}} fn main(){if f()==7{println!(\"ok\");}}"),
        ("typed_dead_negation", "fn f()->i32{-{return 7;0}} fn main(){if f()==7{println!(\"ok\");}}"),
        ("typed_dead_comparison", "fn f()->bool{1=={return false;0}} fn main(){if !f(){println!(\"ok\");}}"),
        ("logical_never", "fn f()->bool{true&&{return false;}} fn main(){if !f(){println!(\"ok\");}}"),
        ("never_local_coercion", "fn f(){let x={return;};let y:bool=x;} fn main(){f();println!(\"ok\");}"),
        ("dead_tail_keeps_type", "fn f()->i32{return 7;9} fn main(){if f()==7{println!(\"ok\");}}"),
        ("never_break_operand", "fn f()->bool{loop{break {return false;};}} fn main(){if !f(){println!(\"ok\");}}"),
        ("typed_dead_break_operand", "fn f()->i32{loop{break {return 7;0};}} fn main(){if f()==7{println!(\"ok\");}}"),
        ("never_unary_not", "fn f()->bool{!{return true;}} fn main(){if f(){println!(\"ok\");}}"),
        ("typed_continue_operand", "fn main(){let mut i=0;while i<3{i=i+1;let x=1+{continue;0};}if i==3{println!(\"ok\");}}"),
    ]
    for name, source in valid:
        yield Case(name, "valid", source, "ok\n")
    invalid = [
        ("never_arithmetic_rhs", "fn f()->i32{1+{return 3;}} fn main(){}"),
        ("never_arithmetic_lhs", "fn f()->i32{({return 3;})+1} fn main(){}"),
        ("never_negation", "fn f()->i32{-{return 3;}} fn main(){}"),
        ("literal_never_equality", "fn f()->bool{1=={return false;}} fn main(){}"),
        ("never_lhs_equality", "fn f()->bool{({return false;})==true} fn main(){}"),
        ("dead_wrong_function_tail", "fn f()->i32{return 3;()} fn main(){}"),
        ("dead_wrong_bool_tail", "fn f()->i32{return 3;false} fn main(){}"),
        ("dead_wrong_argument", "fn g(x:i32){} fn f(){g({return;false});} fn main(){}"),
        ("dead_wrong_local", "fn f(){let x:i32={return;false};} fn main(){}"),
        ("dead_wrong_logical_rhs", "fn f()->bool{true&&{return false;0}} fn main(){}"),
        ("dead_wrong_if_condition", "fn f(){if {return;1}{}} fn main(){}"),
        ("dead_wrong_if_branches", "fn f(){let x=if true{return;false}else{return;0};} fn main(){}"),
        ("dead_wrong_break_type", "fn f(){let x:bool=loop{break {return;0};};} fn main(){}"),
        ("dead_conflicting_breaks", "fn f(){let x=loop{break {return;0};break false;};} fn main(){}"),
        ("dead_nonunit_loop_tail", "fn f(){let x=loop{break 1;false};} fn main(){}"),
    ]
    for name, source in invalid:
        yield Case(name, "invalid", source)
    boundaries = [
        ("known_i32_never_comparison", "fn f()->bool{let x:i32=1;x=={return false;}} fn main(){if !f(){println!(\"ok\");}}"),
        ("bool_never_comparison", "fn f()->bool{true=={return false;}} fn main(){if !f(){println!(\"ok\");}}"),
        ("both_never_comparison", "fn f()->bool{({return false;})==({return true;})} fn main(){if !f(){println!(\"ok\");}}"),
    ]
    for name, source in boundaries:
        yield Case(name, "boundary", source, "ok\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--rustc", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    rustc = args.rustc.absolute()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-never-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, 30)
        if not accepted(version):
            raise SystemExit(f"stock version check failed: {version}")
        record({"record": "environment", "reference": version["stdout"],
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                "runner_sha256": hashlib.sha256(Path(__file__).with_name("module_correctness.py").read_bytes()).hexdigest(),
                "timing_scope": "bounded correctness capture, not a compiler latency benchmark"})
        totals = {"accepted_parity": 0, "invalid_rejected": 0, "unsupported_boundary": 0, "failure": 0}
        for index, case in enumerate(cases()):
            row = run_case(case, index, directory / str(index), binary, rustc, 30, 5)
            record(row)
            totals[row["outcome"]] += 1
            print(f"{case.name}: {row['outcome']}", flush=True)
        record({"record": "summary", **totals, "passed": totals["failure"] == 0})
        if totals["failure"]:
            raise SystemExit(f"{totals['failure']} semantic discrepancies, see {args.output}")


if __name__ == "__main__":
    main()
