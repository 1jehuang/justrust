#!/usr/bin/env python3
"""Bounded E1 lifetime-elision differential corpus, not full Rust compatibility.

Exact frozen stock-classified source bytes/stdout mirror bear's E1 acceptance
corpus. Only free-function signature lifetime elision is claimed.
Implicit autoderef, mixed kinds, methods and placeholder lifetimes
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


ACCEPTANCE_CORPUS_SHA256 = "8245335da3b4dbc4872b01a13e3ab9565d867593d5914b49b68179045195ce26"

# Authoritative frozen bear oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "expected": 7,
    "expected_stdout": "elision oracle ok\n",
    "full_source": "fn id(x:&i32)->&i32{x} fn run()->i32{let x=7;*id(&x)}\nfn main() { if run() == 7 { println!(\"elision oracle ok\"); } }\n",
    "group": "positive",
    "name": "shared_identity",
    "source": "fn id(x:&i32)->&i32{x} fn run()->i32{let x=7;*id(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "elision oracle ok\n",
    "full_source": "fn id(x:&mut i32)->&mut i32{x} fn run()->i32{let mut x=3;let r=id(&mut x);*r=7;x}\nfn main() { if run() == 7 { println!(\"elision oracle ok\"); } }\n",
    "group": "positive",
    "name": "mutable_identity",
    "source": "fn id(x:&mut i32)->&mut i32{x} fn run()->i32{let mut x=3;let r=id(&mut x);*r=7;x}"
  },
  {
    "expected": 7,
    "expected_stdout": "elision oracle ok\n",
    "full_source": "struct S{x:i32} fn get(p:&S)->&i32{&(*p).x} fn run()->i32{let p=S{x:7};*get(&p)}\nfn main() { if run() == 7 { println!(\"elision oracle ok\"); } }\n",
    "group": "positive",
    "name": "shared_projected",
    "source": "struct S{x:i32} fn get(p:&S)->&i32{&(*p).x} fn run()->i32{let p=S{x:7};*get(&p)}"
  },
  {
    "expected": 7,
    "expected_stdout": "elision oracle ok\n",
    "full_source": "fn get(p:&mut(i32,i32))->&mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let r=get(&mut p);*r=7;p.1}\nfn main() { if run() == 7 { println!(\"elision oracle ok\"); } }\n",
    "group": "positive",
    "name": "mutable_projected",
    "source": "fn get(p:&mut(i32,i32))->&mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let r=get(&mut p);*r=7;p.1}"
  },
  {
    "expected": 7,
    "expected_stdout": "elision oracle ok\n",
    "full_source": "fn recur(x:&i32,n:i32)->&i32{if n==0{x}else{recur(x,n-1)}} fn forward(x:&i32)->&i32{recur(x,3)} fn run()->i32{let x=7;*forward(&x)}\nfn main() { if run() == 7 { println!(\"elision oracle ok\"); } }\n",
    "group": "positive",
    "name": "shared_forward_recursive",
    "source": "fn recur(x:&i32,n:i32)->&i32{if n==0{x}else{recur(x,n-1)}} fn forward(x:&i32)->&i32{recur(x,3)} fn run()->i32{let x=7;*forward(&x)}"
  },
  {
    "expected": 7,
    "expected_stdout": "elision oracle ok\n",
    "full_source": "fn recur(x:&mut i32,n:i32)->&mut i32{if n==0{x}else{recur(x,n-1)}} fn forward(x:&mut i32)->&mut i32{recur(x,3)} fn run()->i32{let mut x=3;let r=forward(&mut x);*r=7;x}\nfn main() { if run() == 7 { println!(\"elision oracle ok\"); } }\n",
    "group": "positive",
    "name": "mutable_forward_recursive",
    "source": "fn recur(x:&mut i32,n:i32)->&mut i32{if n==0{x}else{recur(x,n-1)}} fn forward(x:&mut i32)->&mut i32{recur(x,3)} fn run()->i32{let mut x=3;let r=forward(&mut x);*r=7;x}"
  },
  {
    "expected": "E0106",
    "expected_stdout": "",
    "full_source": "fn choose<'a>(x:&'a i32,y:&'a i32,b:bool)->&i32{if b{x}else{y}} fn run()->i32{let x=3;let y=7;*choose(&x,&y,false)}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "repeated_named_shared_region",
    "source": "fn choose<'a>(x:&'a i32,y:&'a i32,b:bool)->&i32{if b{x}else{y}} fn run()->i32{let x=3;let y=7;*choose(&x,&y,false)}"
  },
  {
    "expected": "E0106",
    "expected_stdout": "",
    "full_source": "fn choose<'a>(x:&'a mut i32,y:&'a mut i32,b:bool)->&mut i32{if b{x}else{y}} fn run()->i32{let mut x=3;let mut y=4;let r=choose(&mut x,&mut y,false);*r=7;y}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "repeated_named_mutable_region",
    "source": "fn choose<'a>(x:&'a mut i32,y:&'a mut i32,b:bool)->&mut i32{if b{x}else{y}} fn run()->i32{let mut x=3;let mut y=4;let r=choose(&mut x,&mut y,false);*r=7;y}"
  },
  {
    "expected": 7,
    "expected_stdout": "elision oracle ok\n",
    "full_source": "fn id(n:i32,x:&i32,b:bool,k:usize)->&i32{x} fn run()->i32{let x=7;*id(0,&x,true,1usize)}\nfn main() { if run() == 7 { println!(\"elision oracle ok\"); } }\n",
    "group": "positive",
    "name": "scalar_arguments_ignored",
    "source": "fn id(n:i32,x:&i32,b:bool,k:usize)->&i32{x} fn run()->i32{let x=7;*id(0,&x,true,1usize)}"
  },
  {
    "expected": 7,
    "expected_stdout": "elision oracle ok\n",
    "full_source": "fn pick<'a>(x:&'a i32,y:&bool)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=true;r=pick(&x,&y);}*r}\nfn main() { if run() == 7 { println!(\"elision oracle ok\"); } }\n",
    "group": "positive",
    "name": "explicit_output_independent_fresh_input",
    "source": "fn pick<'a>(x:&'a i32,y:&bool)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=true;r=pick(&x,&y);}*r}"
  },
  {
    "expected": 10,
    "expected_stdout": "elision oracle ok\n",
    "full_source": "fn id(x:&i32)->&i32{x} fn run()->i32{let mut x=3;let r=id(&x);let n=*r;x=7;n+x}\nfn main() { if run() == 10 { println!(\"elision oracle ok\"); } }\n",
    "group": "positive",
    "name": "last_use_then_owner_mutation",
    "source": "fn id(x:&i32)->&i32{x} fn run()->i32{let mut x=3;let r=id(&x);let n=*r;x=7;n+x}"
  },
  {
    "expected": 7,
    "expected_stdout": "elision oracle ok\n",
    "full_source": "fn id(x:&mut i32)->&mut i32{x} fn run()->i32{let mut x=3;let r:&mut i32;{let p=&mut x;r=id(p);}*r=7;x}\nfn main() { if run() == 7 { println!(\"elision oracle ok\"); } }\n",
    "group": "positive",
    "name": "carrier_storage_dead_owner_live",
    "source": "fn id(x:&mut i32)->&mut i32{x} fn run()->i32{let mut x=3;let r:&mut i32;{let p=&mut x;r=id(p);}*r=7;x}"
  },
  {
    "expected": "E0106",
    "expected_stdout": "",
    "full_source": "fn bad()->&i32{loop{}} fn run()->i32{7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "zero_input_output_lifetime",
    "source": "fn bad()->&i32{loop{}} fn run()->i32{7}"
  },
  {
    "expected": "E0106",
    "expected_stdout": "",
    "full_source": "fn bad(x:&i32,y:&i32)->&i32{x} fn run()->i32{7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "two_anonymous_input_ambiguity",
    "source": "fn bad(x:&i32,y:&i32)->&i32{x} fn run()->i32{7}"
  },
  {
    "expected": "E0106",
    "expected_stdout": "",
    "full_source": "fn bad<'a>(x:&'a i32,y:&i32)->&i32{x} fn run()->i32{7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "explicit_and_fresh_output_ambiguity",
    "source": "fn bad<'a>(x:&'a i32,y:&i32)->&i32{x} fn run()->i32{7}"
  },
  {
    "expected": "E0621",
    "expected_stdout": "",
    "full_source": "fn bad<'a>(x:&'a i32,y:&i32)->&'a i32{y} fn run()->i32{7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "fresh_input_cannot_satisfy_named_output",
    "source": "fn bad<'a>(x:&'a i32,y:&i32)->&'a i32{y} fn run()->i32{7}"
  },
  {
    "expected": "E0515",
    "expected_stdout": "",
    "full_source": "fn bad(x:&i32)->&i32{let y=7;&y} fn run()->i32{7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "local_owner_return",
    "source": "fn bad(x:&i32)->&i32{let y=7;&y} fn run()->i32{7}"
  },
  {
    "expected": "E0597",
    "expected_stdout": "",
    "full_source": "fn pick<'a>(x:&'a bool,y:&'a i32)->&'a i32{y} fn run()->i32{let y=7;let r:&i32;{let x=true;r=pick(&x,&y);}*r}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "same_region_dependency_storage_pin",
    "source": "fn pick<'a>(x:&'a bool,y:&'a i32)->&'a i32{y} fn run()->i32{let y=7;let r:&i32;{let x=true;r=pick(&x,&y);}*r}"
  },
  {
    "expected": 7,
    "expected_stdout": "elision oracle ok\n",
    "full_source": "struct S{x:i32} fn get(p:&S)->&i32{&p.x} fn run()->i32{let p=S{x:7};*get(&p)}\nfn main() { if run() == 7 { println!(\"elision oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "implicit_autoderef_projected",
    "source": "struct S{x:i32} fn get(p:&S)->&i32{&p.x} fn run()->i32{let p=S{x:7};*get(&p)}"
  },
  {
    "expected": 7,
    "expected_stdout": "elision oracle ok\n",
    "full_source": "fn pick<'a>(x:&'a mut i32,y:&'a bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let y=true;let r=pick(&mut x,&y);*r=7;x}\nfn main() { if run() == 7 { println!(\"elision oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "mixed_kind_inputs",
    "source": "fn pick<'a>(x:&'a mut i32,y:&'a bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let y=true;let r=pick(&mut x,&y);*r=7;x}"
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
            prefix="jr-lifetime-elision-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "borrowing-e1-lifetime-elision-v1",
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
            raise SystemExit(f"{totals['failure']} E1 lifetime-elision discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
