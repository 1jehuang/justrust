#!/usr/bin/env python3
"""Bounded B3b mutable-reference-return differential corpus, not full Rust compatibility.

Exact frozen stock-classified source bytes/stdout mirror bear's B3b acceptance
corpus. Only explicit-region whole mutable-parameter reference returns are
claimed. Shared-input/projected/aggregate returns and broader elision
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


ACCEPTANCE_CORPUS_SHA256 = "5049cb8e2f69028b9df552a92db2e3d8640b4ea6d40e68e372d516ab7cdc1b11"

# Original immutable acceptance artifact hash retained above.
# E1 native-pass promotion: elided_mutable_result is now positive; source/stdout/value unchanged.
# Authoritative bear source bytes. full_source has no added wrapper.
# E2 native-pass promotion: projected_mutable_result is positive; source/stdout/value unchanged.
# Original immutable acceptance artifact hash above is retained.
# E3 classification promotion: shared_input_mutable_result; exact source/stdout/value retained.
# Original immutable acceptance artifact hash above is unchanged.
# Generic-reference native-pass promotion: computed_call_assignment is positive;
# exact source/stdout/value retained after stock and native workers 1/8 parity.
FIXTURES = json.loads(r'''
[
  {
    "group": "positive",
    "name": "identity_scalar_write",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);*q=7;x}",
    "expected": 7,
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);*q=7;x}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "full64_storage_write",
    "source": "fn id<'a>(r:&'a mut usize)->&'a mut usize{r} fn run()->i32{let mut x=3usize;let q=id(&mut x);*q=18446744073709551615usize;if x==18446744073709551615usize{7}else{0}}",
    "expected": 7,
    "full_source": "fn id<'a>(r:&'a mut usize)->&'a mut usize{r} fn run()->i32{let mut x=3usize;let q=id(&mut x);*q=18446744073709551615usize;if x==18446744073709551615usize{7}else{0}}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "aggregate_storage_write",
    "source": "struct S{x:i32,y:i32} fn id<'a>(r:&'a mut S)->&'a mut S{r} fn run()->i32{let mut s=S{x:3,y:4};let q=id(&mut s);(*q).x=7;(*q).y=8;s.x+s.y}",
    "expected": 15,
    "full_source": "struct S{x:i32,y:i32} fn id<'a>(r:&'a mut S)->&'a mut S{r} fn run()->i32{let mut s=S{x:3,y:4};let q=id(&mut s);(*q).x=7;(*q).y=8;s.x+s.y}\nfn main() { if run() == 15 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "two_candidates_both_branches",
    "source": "fn choose<'a>(x:&'a mut i32,y:&'a mut i32,b:bool)->&'a mut i32{if b{x}else{y}} fn run()->i32{let mut x=3;let mut y=4;let q=choose(&mut x,&mut y,true);*q=7;let r=choose(&mut x,&mut y,false);*r=8;x+y}",
    "expected": 15,
    "full_source": "fn choose<'a>(x:&'a mut i32,y:&'a mut i32,b:bool)->&'a mut i32{if b{x}else{y}} fn run()->i32{let mut x=3;let mut y=4;let q=choose(&mut x,&mut y,true);*q=7;let r=choose(&mut x,&mut y,false);*r=8;x+y}\nfn main() { if run() == 15 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "original_parameter_index",
    "source": "fn pick<'a>(n:i32,x:&'a mut i32,y:&'a mut bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let mut y=true;let q=pick(0,&mut x,&mut y);*q=7;x}",
    "expected": 7,
    "full_source": "fn pick<'a>(n:i32,x:&'a mut i32,y:&'a mut bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let mut y=true;let q=pick(0,&mut x,&mut y);*q=7;x}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "expected_return_reborrow",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{let q:&mut i32=r;q} fn run()->i32{let mut x=3;let q=id(&mut x);*q=7;x}",
    "expected": 7,
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{let q:&mut i32=r;q} fn run()->i32{let mut x=3;let q=id(&mut x);*q=7;x}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "explicit_return_reborrow",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{&mut *r} fn run()->i32{let mut x=3;let q=id(&mut x);*q=7;x}",
    "expected": 7,
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{&mut *r} fn run()->i32{let mut x=3;let q=id(&mut x);*q=7;x}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "forward_recursive",
    "source": "fn recur<'a>(r:&'a mut i32,n:i32)->&'a mut i32{if n==0{r}else{recur(r,n-1)}} fn forward<'a>(r:&'a mut i32)->&'a mut i32{recur(r,3)} fn run()->i32{let mut x=3;let q=forward(&mut x);*q=7;x}",
    "expected": 7,
    "full_source": "fn recur<'a>(r:&'a mut i32,n:i32)->&'a mut i32{if n==0{r}else{recur(r,n-1)}} fn forward<'a>(r:&'a mut i32)->&'a mut i32{recur(r,3)} fn run()->i32{let mut x=3;let q=forward(&mut x);*q=7;x}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "result_handle_move",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);let moved=q;*moved=7;x}",
    "expected": 7,
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);let moved=q;*moved=7;x}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "parent_resumes_after_last_use",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(r);*q=7;*r=*r+1;*r}",
    "expected": 8,
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(r);*q=7;*r=*r+1;*r}\nfn main() { if run() == 8 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "shared_child_reborrow",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);let child=&*q;let n=*child;*q=7;n+*q}",
    "expected": 10,
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);let child=&*q;let n=*child;*q=7;n+*q}\nfn main() { if run() == 10 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutable_child_reborrow",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);let child=&mut *q;*child=7;*q=*q+1;*q}",
    "expected": 8,
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);let child=&mut *q;*child=7;*q=*q+1;*q}\nfn main() { if run() == 8 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "parent_carrier_storage_dead",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q:&mut i32;{let r=&mut x;q=id(r);}*q=7;x}",
    "expected": 7,
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q:&mut i32;{let r=&mut x;q=id(r);}*q=7;x}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "independent_short_input",
    "source": "fn pick<'a,'b>(x:&'a mut i32,y:&'b mut bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let q:&mut i32;{let mut y=true;q=pick(&mut x,&mut y);}*q=7;x}",
    "expected": 7,
    "full_source": "fn pick<'a,'b>(x:&'a mut i32,y:&'b mut bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let q:&mut i32;{let mut y=true;q=pick(&mut x,&mut y);}*q=7;x}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutate_nonreturned_input",
    "source": "fn pick<'a,'b>(x:&'a mut i32,y:&'b mut i32)->&'a mut i32{*y=8;x} fn run()->i32{let mut x=3;let mut y=4;let q=pick(&mut x,&mut y);*q=7;x+y}",
    "expected": 15,
    "full_source": "fn pick<'a,'b>(x:&'a mut i32,y:&'b mut i32)->&'a mut i32{*y=8;x} fn run()->i32{let mut x=3;let mut y=4;let q=pick(&mut x,&mut y);*q=7;x+y}\nfn main() { if run() == 15 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "direct_call_copy_read",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=7;*id(&mut x)}",
    "expected": 7,
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=7;*id(&mut x)}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "negative",
    "name": "overlapping_mutable_actuals",
    "source": "fn choose<'a>(x:&'a mut i32,y:&'a mut i32)->&'a mut i32{x} fn run()->i32{let mut x=3;let q=choose(&mut x,&mut x);*q}",
    "expected": "E0499",
    "full_source": "fn choose<'a>(x:&'a mut i32,y:&'a mut i32)->&'a mut i32{x} fn run()->i32{let mut x=3;let q=choose(&mut x,&mut x);*q}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "result_used_after_move",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);let moved=q;*moved=7;*q}",
    "expected": "E0382",
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);let moved=q;*moved=7;*q}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "live_child_parent_read",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(r);let n=*r;*q+n}",
    "expected": "E0503",
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(r);let n=*r;*q+n}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "live_child_parent_write",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(r);*r=7;*q}",
    "expected": "E0506",
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(r);*r=7;*q}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "live_child_parent_move",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(&mut *r);let moved=r;*q+*moved}",
    "expected": "E0505",
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(&mut *r);let moved=r;*q+*moved}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "live_child_parent_borrow",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(r);let s=&mut *r;*q+*s}",
    "expected": "E0499",
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(r);let s=&mut *r;*q+*s}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "owned_local_return",
    "source": "fn bad<'a>(r:&'a mut i32)->&'a mut i32{let mut y=7;&mut y} fn run()->i32{7}",
    "expected": "E0515",
    "full_source": "fn bad<'a>(r:&'a mut i32)->&'a mut i32{let mut y=7;&mut y} fn run()->i32{7}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "unused_bad_callee",
    "source": "fn bad<'a>()->&'a mut i32{let mut y=7;&mut y} fn run()->i32{7}",
    "expected": "E0515",
    "full_source": "fn bad<'a>()->&'a mut i32{let mut y=7;&mut y} fn run()->i32{7}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "missing_universal_outlives",
    "source": "fn bad<'a,'b>(x:&'a mut i32,y:&'b mut i32)->&'a mut i32{y} fn run()->i32{7}",
    "expected": null,
    "full_source": "fn bad<'a,'b>(x:&'a mut i32,y:&'b mut i32)->&'a mut i32{y} fn run()->i32{7}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "same_region_different_pointee_dependency",
    "source": "fn pick<'a>(x:&'a mut i32,y:&'a mut bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let q:&mut i32;{let mut y=true;q=pick(&mut x,&mut y);}*q}",
    "expected": "E0597",
    "full_source": "fn pick<'a>(x:&'a mut i32,y:&'a mut bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let q:&mut i32;{let mut y=true;q=pick(&mut x,&mut y);}*q}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "positive",
    "name": "projected_mutable_result",
    "source": "struct S{x:i32} fn get<'a>(x:&'a mut S)->&'a mut i32{&mut x.x} fn run()->i32{let mut s=S{x:3};let q=get(&mut s);*q=7;s.x}",
    "expected": 7,
    "full_source": "struct S{x:i32} fn get<'a>(x:&'a mut S)->&'a mut i32{&mut x.x} fn run()->i32{let mut s=S{x:3};let q=get(&mut s);*q=7;s.x}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "shared_input_mutable_result",
    "source": "fn pick<'a>(x:&'a mut i32,y:&'a bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let y=true;let q=pick(&mut x,&y);*q=7;x}",
    "expected": 7,
    "full_source": "fn pick<'a>(x:&'a mut i32,y:&'a bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let y=true;let q=pick(&mut x,&y);*q=7;x}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "elided_mutable_result",
    "source": "fn id(r:&mut i32)->&mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);*q=7;x}",
    "expected": 7,
    "full_source": "fn id(r:&mut i32)->&mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);*q=7;x}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "computed_call_assignment",
    "source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;*id(&mut x)=7;x}",
    "expected": 7,
    "full_source": "fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;*id(&mut x)=7;x}\nfn main() { if run() == 7 { println!(\"mutable returns oracle ok\"); } }\n",
    "expected_stdout": "mutable returns oracle ok\n"
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
            prefix="jr-mutable-reference-returns-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "borrowing-b3b-mutable-reference-returns-v1",
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
            raise SystemExit(f"{totals['failure']} B3b reference-return discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
