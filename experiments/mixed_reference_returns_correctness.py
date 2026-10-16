#!/usr/bin/env python3
"""Bounded E3 mixed-reference-returns differential corpus, not full Rust compatibility.

Exact frozen stock-classified source bytes/stdout mirror bear's E3 acceptance
corpus. Only mixed-capability reference-result inputs are claimed.
Explicit mutable-borrow coercions and general coercion expansion
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


ACCEPTANCE_CORPUS_SHA256 = "0cc633f928bdf504c092c6124eafa4f4edb20b200575c59470bf71e8466497ae"

# Authoritative frozen bear oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "group": "positive",
    "name": "shared_result_from_mutable_input",
    "source": "fn get(x:&mut i32)->&i32{&*x} fn run()->i32{let mut x=7;*get(&mut x)}",
    "expected": 7,
    "full_source": "fn get(x:&mut i32)->&i32{&*x} fn run()->i32{let mut x=7;*get(&mut x)}\nfn main() { if run() == 7 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "shared_target_unrelated_mutable_input",
    "source": "fn get<'a,'b>(x:&'a i32,y:&'b mut bool)->&'a i32{x} fn run()->i32{let x=7;let mut y=true;*get(&x,&mut y)}",
    "expected": 7,
    "full_source": "fn get<'a,'b>(x:&'a i32,y:&'b mut bool)->&'a i32{x} fn run()->i32{let x=7;let mut y=true;*get(&x,&mut y)}\nfn main() { if run() == 7 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutable_target_shared_result_mixed_inputs",
    "source": "fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a i32{&*x} fn run()->i32{let mut x=7;let y=true;*get(&mut x,&y)}",
    "expected": 7,
    "full_source": "fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a i32{&*x} fn run()->i32{let mut x=7;let y=true;*get(&mut x,&y)}\nfn main() { if run() == 7 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "shared_projected_from_mutable",
    "source": "fn get(x:&mut(i32,i32))->&i32{&x.1} fn run()->i32{let mut x=(3,7);*get(&mut x)}",
    "expected": 7,
    "full_source": "fn get(x:&mut(i32,i32))->&i32{&x.1} fn run()->i32{let mut x=(3,7);*get(&mut x)}\nfn main() { if run() == 7 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutable_result_unrelated_shared_input",
    "source": "fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let y=true;let r=get(&mut x,&y);*r=7;x}",
    "expected": 7,
    "full_source": "fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let y=true;let r=get(&mut x,&y);*r=7;x}\nfn main() { if run() == 7 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutable_projected_mixed_inputs",
    "source": "struct S{x:i32} fn get<'a,'b>(x:&'a mut S,y:&'b bool)->&'a mut i32{&mut x.x} fn run()->i32{let mut x=S{x:3};let y=true;let r=get(&mut x,&y);*r=7;x.x}",
    "expected": 7,
    "full_source": "struct S{x:i32} fn get<'a,'b>(x:&'a mut S,y:&'b bool)->&'a mut i32{&mut x.x} fn run()->i32{let mut x=S{x:3};let y=true;let r=get(&mut x,&y);*r=7;x.x}\nfn main() { if run() == 7 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "independent_short_mutable_dependency",
    "source": "fn get<'a,'b>(x:&'a i32,y:&'b mut bool)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let mut y=true;r=get(&x,&mut y);}*r}",
    "expected": 7,
    "full_source": "fn get<'a,'b>(x:&'a i32,y:&'b mut bool)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let mut y=true;r=get(&x,&mut y);}*r}\nfn main() { if run() == 7 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "independent_short_shared_dependency",
    "source": "fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let r:&mut i32;{let y=true;r=get(&mut x,&y);}*r=7;x}",
    "expected": 7,
    "full_source": "fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let r:&mut i32;{let y=true;r=get(&mut x,&y);}*r=7;x}\nfn main() { if run() == 7 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "shared_result_mixed_candidate_branch",
    "source": "fn get<'a>(x:&'a i32,y:&'a mut i32,b:bool)->&'a i32{if b{x}else{&*y}} fn run()->i32{let x=3;let mut y=7;*get(&x,&mut y,false)}",
    "expected": 7,
    "full_source": "fn get<'a>(x:&'a i32,y:&'a mut i32,b:bool)->&'a i32{if b{x}else{&*y}} fn run()->i32{let x=3;let mut y=7;*get(&x,&mut y,false)}\nfn main() { if run() == 7 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "recursive_mixed_shared_return",
    "source": "fn recur<'a,'b>(x:&'a mut i32,y:&'b bool,n:i32)->&'a i32{if n==0{&*x}else{recur(x,y,n-1)}} fn run()->i32{let mut x=7;let y=true;*recur(&mut x,&y,3)}",
    "expected": 7,
    "full_source": "fn recur<'a,'b>(x:&'a mut i32,y:&'b bool,n:i32)->&'a i32{if n==0{&*x}else{recur(x,y,n-1)}} fn run()->i32{let mut x=7;let y=true;*recur(&mut x,&y,3)}\nfn main() { if run() == 7 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "shared_child_parent_read_lastuse_resume",
    "source": "fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a i32{&*x} fn run()->i32{let mut x=3;let y=true;let parent=&mut x;let r=get(parent,&y);let n=*r+*parent;*parent=7;n+*parent}",
    "expected": 13,
    "full_source": "fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a i32{&*x} fn run()->i32{let mut x=3;let y=true;let parent=&mut x;let r=get(parent,&y);let n=*r+*parent;*parent=7;n+*parent}\nfn main() { if run() == 13 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutate_unrelated_nonreturned_input",
    "source": "fn get<'a,'b>(x:&'a i32,y:&'b mut i32)->&'a i32{*y=8;x} fn run()->i32{let x=7;let mut y=3;let r=get(&x,&mut y);*r+y}",
    "expected": 15,
    "full_source": "fn get<'a,'b>(x:&'a i32,y:&'b mut i32)->&'a i32{*y=8;x} fn run()->i32{let x=7;let mut y=3;let r=get(&x,&mut y);*r+y}\nfn main() { if run() == 15 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "negative",
    "name": "shared_input_cannot_upgrade_mutable",
    "source": "fn bad<'a>(x:&'a i32)->&'a mut i32{&mut *x} fn run()->i32{7}",
    "expected": "E0596",
    "full_source": "fn bad<'a>(x:&'a i32)->&'a mut i32{&mut *x} fn run()->i32{7}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "overlapping_mixed_actuals",
    "source": "fn get<'a>(x:&'a mut i32,y:&'a i32)->&'a i32{&*x} fn run()->i32{let mut x=3;*get(&mut x,&x)}",
    "expected": "E0502",
    "full_source": "fn get<'a>(x:&'a mut i32,y:&'a i32)->&'a i32{&*x} fn run()->i32{let mut x=3;*get(&mut x,&x)}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "shared_result_same_region_other_pointee_pin",
    "source": "fn get<'a>(x:&'a mut i32,y:&'a bool)->&'a i32{&*x} fn run()->i32{let mut x=7;let r:&i32;{let y=true;r=get(&mut x,&y);}*r}",
    "expected": "E0597",
    "full_source": "fn get<'a>(x:&'a mut i32,y:&'a bool)->&'a i32{&*x} fn run()->i32{let mut x=7;let r:&i32;{let y=true;r=get(&mut x,&y);}*r}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "mutable_result_same_region_shared_pin",
    "source": "fn get<'a>(x:&'a mut i32,y:&'a bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let r:&mut i32;{let y=true;r=get(&mut x,&y);}*r}",
    "expected": "E0597",
    "full_source": "fn get<'a>(x:&'a mut i32,y:&'a bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let r:&mut i32;{let y=true;r=get(&mut x,&y);}*r}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "live_shared_result_parent_write",
    "source": "fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a i32{&*x} fn run()->i32{let mut x=3;let y=true;let parent=&mut x;let r=get(parent,&y);*parent=7;*r}",
    "expected": "E0506",
    "full_source": "fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a i32{&*x} fn run()->i32{let mut x=3;let y=true;let parent=&mut x;let r=get(parent,&y);*parent=7;*r}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "live_mutable_result_parent_read",
    "source": "fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let y=true;let parent=&mut x;let r=get(parent,&y);let n=*parent;*r+n}",
    "expected": "E0503",
    "full_source": "fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let y=true;let parent=&mut x;let r=get(parent,&y);let n=*parent;*r+n}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "legalunsupported",
    "name": "explicit_mutable_borrow_to_shared_result",
    "source": "fn get(x:&mut i32)->&i32{&mut *x} fn run()->i32{let mut x=7;*get(&mut x)}",
    "expected": 7,
    "full_source": "fn get(x:&mut i32)->&i32{&mut *x} fn run()->i32{let mut x=7;*get(&mut x)}\nfn main() { if run() == 7 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "projected_mutable_borrow_to_shared_result",
    "source": "struct S{x:i32} fn get(x:&mut S)->&i32{&mut x.x} fn run()->i32{let mut x=S{x:7};*get(&mut x)}",
    "expected": 7,
    "full_source": "struct S{x:i32} fn get(x:&mut S)->&i32{&mut x.x} fn run()->i32{let mut x=S{x:7};*get(&mut x)}\nfn main() { if run() == 7 { println!(\"mixed returns oracle ok\"); } }\n",
    "expected_stdout": "mixed returns oracle ok\n"
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
            prefix="jr-mixed-reference-returns-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "borrowing-e3-mixed-reference-returns-v1",
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
            raise SystemExit(f"{totals['failure']} E3 mixed-reference-returns discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
