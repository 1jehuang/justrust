#!/usr/bin/env python3
"""Bounded bitwise differential corpus, not full Rust compatibility.

Exact frozen stock-classified source bytes/stdout mirror bear's bitwise acceptance
corpus. Integer and eager boolean &, |, ^ are covered, including builtin compound
assignments. 128-bit integers remain an explicit boundary. Valid runs require zero exit, exact
stdout and empty stderr. Reuse module_correctness Case/run_case/process and
bounded capture, with supplied-native workers 1/8 check/build. Never build a binary
here. No trap exceptions apply.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "1b3f06ed4dae9915ffb67625f18898193c12feb21f396cd1385b63c8dba8d7e9"

# Native-pass promotion: compound_assignment is positive, source/stdout/value unchanged.
# Original immutable acceptance artifact hash above is retained.
# Authoritative frozen bear oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn bits(a:i8,b:i8)->i8{(a&b)|(a^b)}fn run()->i32{let a:i8=10;let b:i8=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1i8 & a)==a{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "operations_i8",
    "source": "fn bits(a:i8,b:i8)->i8{(a&b)|(a^b)}fn run()->i32{let a:i8=10;let b:i8=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1i8 & a)==a{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn bits(a:i16,b:i16)->i16{(a&b)|(a^b)}fn run()->i32{let a:i16=10;let b:i16=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1i16 & a)==a{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "operations_i16",
    "source": "fn bits(a:i16,b:i16)->i16{(a&b)|(a^b)}fn run()->i32{let a:i16=10;let b:i16=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1i16 & a)==a{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn bits(a:i32,b:i32)->i32{(a&b)|(a^b)}fn run()->i32{let a:i32=10;let b:i32=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1i32 & a)==a{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "operations_i32",
    "source": "fn bits(a:i32,b:i32)->i32{(a&b)|(a^b)}fn run()->i32{let a:i32=10;let b:i32=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1i32 & a)==a{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn bits(a:i64,b:i64)->i64{(a&b)|(a^b)}fn run()->i32{let a:i64=10;let b:i64=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1i64 & a)==a{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "operations_i64",
    "source": "fn bits(a:i64,b:i64)->i64{(a&b)|(a^b)}fn run()->i32{let a:i64=10;let b:i64=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1i64 & a)==a{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn bits(a:isize,b:isize)->isize{(a&b)|(a^b)}fn run()->i32{let a:isize=10;let b:isize=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1isize & a)==a{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "operations_isize",
    "source": "fn bits(a:isize,b:isize)->isize{(a&b)|(a^b)}fn run()->i32{let a:isize=10;let b:isize=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1isize & a)==a{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn bits(a:u8,b:u8)->u8{(a&b)|(a^b)}fn run()->i32{let a:u8=10;let b:u8=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "operations_u8",
    "source": "fn bits(a:u8,b:u8)->u8{(a&b)|(a^b)}fn run()->i32{let a:u8=10;let b:u8=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn bits(a:u16,b:u16)->u16{(a&b)|(a^b)}fn run()->i32{let a:u16=10;let b:u16=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "operations_u16",
    "source": "fn bits(a:u16,b:u16)->u16{(a&b)|(a^b)}fn run()->i32{let a:u16=10;let b:u16=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn bits(a:u32,b:u32)->u32{(a&b)|(a^b)}fn run()->i32{let a:u32=10;let b:u32=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "operations_u32",
    "source": "fn bits(a:u32,b:u32)->u32{(a&b)|(a^b)}fn run()->i32{let a:u32=10;let b:u32=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn bits(a:u64,b:u64)->u64{(a&b)|(a^b)}fn run()->i32{let a:u64=10;let b:u64=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "operations_u64",
    "source": "fn bits(a:u64,b:u64)->u64{(a&b)|(a^b)}fn run()->i32{let a:u64=10;let b:u64=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn bits(a:usize,b:usize)->usize{(a&b)|(a^b)}fn run()->i32{let a:usize=10;let b:usize=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "operations_usize",
    "source": "fn bits(a:usize,b:usize)->usize{(a&b)|(a^b)}fn run()->i32{let a:usize=10;let b:usize=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn run()->i32{if (true&true) && !(true&false) && (false|true) && !(false|false) && (true^false) && !(true^true){7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "boolean_eager_truth_tables",
    "source": "fn run()->i32{if (true&true) && !(true&false) && (false|true) && !(false|false) && (true^false) && !(true^true){7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn left(p:&mut i32)->bool{*p=*p*10+1;false}fn right(p:&mut i32)->bool{*p=*p*10+2;true}fn run()->i32{let mut n=0;let a=left(&mut n)&right(&mut n);let b=right(&mut n)|left(&mut n);let c=left(&mut n)^right(&mut n);if n==122112 && !a && b && c{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "eager_once_left_to_right",
    "source": "fn left(p:&mut i32)->bool{*p=*p*10+1;false}fn right(p:&mut i32)->bool{*p=*p*10+2;true}fn run()->i32{let mut n=0;let a=left(&mut n)&right(&mut n);let b=right(&mut n)|left(&mut n);let c=left(&mut n)^right(&mut n);if n==122112 && !a && b && c{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn run()->i32{let x=1u16|2u16^3u16&1u16;let y=15u8^7u8^3u8;if x==3u16 && y==11u8{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "precedence_and_xor_chain",
    "source": "fn run()->i32{let x=1u16|2u16^3u16&1u16;let y=15u8^7u8^3u8;if x==3u16 && y==11u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "struct S{x:u64,y:i64}fn run()->i32{let s=S{x:18446744073709551615u64,y:-9223372036854775808i64};let r=&s;let p=(r.x&9223372036854775808u64,r.y|1i64);if p.0==9223372036854775808u64 && p.1 == -9223372036854775807i64{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "high64_reference_transport",
    "source": "struct S{x:u64,y:i64}fn run()->i32{let s=S{x:18446744073709551615u64,y:-9223372036854775808i64};let r=&s;let p=(r.x&9223372036854775808u64,r.y|1i64);if p.0==9223372036854775808u64 && p.1 == -9223372036854775807i64{7}else{0}}"
  },
  {
    "expected": "E0308",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let a=1u8;let b=2u16;let x=a&b;7}\nfn main(){run();}",
    "group": "negative",
    "name": "mixed_integer_types",
    "source": "fn run()->i32{let a=1u8;let b=2u16;let x=a&b;7}"
  },
  {
    "expected": "E0277",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=true|1i32;7}\nfn main(){run();}",
    "group": "negative",
    "name": "mixed_boolean_integer",
    "source": "fn run()->i32{let x=true|1i32;7}"
  },
  {
    "expected": "overflowing_literals",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=256u8^1u8;7}\nfn main(){run();}",
    "group": "negative",
    "name": "out_of_range_operand",
    "source": "fn run()->i32{let x=256u8^1u8;7}"
  },
  {
    "expected": "E0308",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=1i32^2u32;7}\nfn main(){run();}",
    "group": "negative",
    "name": "signed_unsigned_same_width",
    "source": "fn run()->i32{let x=1i32^2u32;7}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn run()->i32{let mut x=15u8;x&=7u8;x|=8u8;x^=8u8;if x==7u8{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "positive",
    "name": "compound_assignment",
    "source": "fn run()->i32{let mut x=15u8;x&=7u8;x|=8u8;x^=8u8;if x==7u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "bitwise oracle ok\n",
    "full_source": "fn run()->i32{let x=1u128|2u128;let y=-1i128&7i128;if x==3u128 && y==7i128{7}else{0}}\nfn main(){if run()==7{println!(\"bitwise oracle ok\");}}",
    "group": "legalunsupported",
    "name": "integer128_bitwise",
    "source": "fn run()->i32{let x=1u128|2u128;let y=-1i128&7i128;if x==3u128 && y==7i128{7}else{0}}"
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
            prefix="jr-bitwise-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "bitwise-v1",
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
            raise SystemExit(f"{totals['failure']} bitwise discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
