#!/usr/bin/env python3
"""Bounded projected-reference-return differential corpus, not full Rust compatibility.

Exact frozen stock-classified source bytes/stdout mirror bear's projected-input acceptance
corpus. Only explicit-region static input-path reference returns are
claimed. Dynamic/enum-payload/aggregate reference returns and broader elision
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


ACCEPTANCE_CORPUS_SHA256 = "bd1c90a96f02023841f1ee68c4d8b0d863fea94a5f2e14b8118a3dc2ce1bb207"

# Authoritative frozen bear oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "group": "positive",
    "name": "shared_tuple_projection",
    "source": "fn get<'a>(p:&'a(i32,i32))->&'a i32{&(*p).1} fn run()->i32{let p=(3,7);*get(&p)}",
    "expected": 7,
    "full_source": "fn get<'a>(p:&'a(i32,i32))->&'a i32{&(*p).1} fn run()->i32{let p=(3,7);*get(&p)}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutable_tuple_projection",
    "source": "fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let r=get(&mut p);*r=7;p.1}",
    "expected": 7,
    "full_source": "fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let r=get(&mut p);*r=7;p.1}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "shared_struct_projection",
    "source": "struct S{x:i32,y:i32} fn get<'a>(p:&'a S)->&'a i32{&(*p).y} fn run()->i32{let p=S{x:3,y:7};*get(&p)}",
    "expected": 7,
    "full_source": "struct S{x:i32,y:i32} fn get<'a>(p:&'a S)->&'a i32{&(*p).y} fn run()->i32{let p=S{x:3,y:7};*get(&p)}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutable_struct_projection",
    "source": "struct S{x:i32,y:i32} fn get<'a>(p:&'a mut S)->&'a mut i32{&mut (*p).y} fn run()->i32{let mut p=S{x:3,y:4};let r=get(&mut p);*r=7;p.y}",
    "expected": 7,
    "full_source": "struct S{x:i32,y:i32} fn get<'a>(p:&'a mut S)->&'a mut i32{&mut (*p).y} fn run()->i32{let mut p=S{x:3,y:4};let r=get(&mut p);*r=7;p.y}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "shared_constant_array_projection",
    "source": "fn get<'a>(p:&'a[i32;2])->&'a i32{&(*p)[1]} fn run()->i32{let p=[3,7];*get(&p)}",
    "expected": 7,
    "full_source": "fn get<'a>(p:&'a[i32;2])->&'a i32{&(*p)[1]} fn run()->i32{let p=[3,7];*get(&p)}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutable_constant_array_projection",
    "source": "fn get<'a>(p:&'a mut[i32;2])->&'a mut i32{&mut (*p)[1]} fn run()->i32{let mut p=[3,4];let r=get(&mut p);*r=7;p[1]}",
    "expected": 7,
    "full_source": "fn get<'a>(p:&'a mut[i32;2])->&'a mut i32{&mut (*p)[1]} fn run()->i32{let mut p=[3,4];let r=get(&mut p);*r=7;p[1]}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "nested_static_projection",
    "source": "struct S{v:([i32;2],bool)} fn get<'a>(p:&'a S)->&'a i32{&(*p).v.0[1]} fn run()->i32{let p=S{v:([3,7],true)};*get(&p)}",
    "expected": 7,
    "full_source": "struct S{v:([i32;2],bool)} fn get<'a>(p:&'a S)->&'a i32{&(*p).v.0[1]} fn run()->i32{let p=S{v:([3,7],true)};*get(&p)}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "opaque_enum_whole_leaf",
    "source": "enum E{A,B} struct S{e:E} fn get<'a>(p:&'a S)->&'a E{&(*p).e} fn run()->i32{let p=S{e:E::A};let r=get(&p);let _=r;7}",
    "expected": 7,
    "full_source": "enum E{A,B} struct S{e:E} fn get<'a>(p:&'a S)->&'a E{&(*p).e} fn run()->i32{let p=S{e:E::A};let r=get(&p);let _=r;7}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "private_field_public_getter",
    "source": "mod m{pub struct S{x:i32} pub fn make()->S{S{x:7}} pub fn get<'a>(p:&'a S)->&'a i32{&(*p).x}} fn run()->i32{let p=m::make();*m::get(&p)}",
    "expected": 7,
    "full_source": "mod m{pub struct S{x:i32} pub fn make()->S{S{x:7}} pub fn get<'a>(p:&'a S)->&'a i32{&(*p).x}} fn run()->i32{let p=m::make();*m::get(&p)}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "correlated_argument_layouts",
    "source": "fn get<'a>(n:i32,x:&'a(i32,bool),y:&'a(bool,i32),b:bool)->&'a i32{if b{&(*x).0}else{&(*y).1}} fn run()->i32{let x=(3,true);let y=(false,7);*get(0,&x,&y,true)*10+*get(0,&x,&y,false)}",
    "expected": 37,
    "full_source": "fn get<'a>(n:i32,x:&'a(i32,bool),y:&'a(bool,i32),b:bool)->&'a i32{if b{&(*x).0}else{&(*y).1}} fn run()->i32{let x=(3,true);let y=(false,7);*get(0,&x,&y,true)*10+*get(0,&x,&y,false)}\nfn main() { if run() == 37 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "nominal_leaf_identity",
    "source": "struct A{x:i32} struct B{x:i32} struct S{a:A,b:B} fn get<'a>(p:&'a S)->&'a B{&(*p).b} fn run()->i32{let p=S{a:A{x:3},b:B{x:7}};(*get(&p)).x}",
    "expected": 7,
    "full_source": "struct A{x:i32} struct B{x:i32} struct S{a:A,b:B} fn get<'a>(p:&'a S)->&'a B{&(*p).b} fn run()->i32{let p=S{a:A{x:3},b:B{x:7}};(*get(&p)).x}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "forward_recursive_projection",
    "source": "fn get<'a>(p:&'a(i32,i32),n:i32)->&'a i32{if n==0{&(*p).1}else{get(p,n-1)}} fn forward<'a>(p:&'a(i32,i32))->&'a i32{get(p,3)} fn run()->i32{let p=(3,7);*forward(&p)}",
    "expected": 7,
    "full_source": "fn get<'a>(p:&'a(i32,i32),n:i32)->&'a i32{if n==0{&(*p).1}else{get(p,n-1)}} fn forward<'a>(p:&'a(i32,i32))->&'a i32{get(p,3)} fn run()->i32{let p=(3,7);*forward(&p)}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "high64_mutable_projection",
    "source": "struct S{x:usize} fn get<'a>(p:&'a mut S)->&'a mut usize{&mut (*p).x} fn run()->i32{let mut p=S{x:3usize};let r=get(&mut p);*r=18446744073709551615usize;if p.x==18446744073709551615usize{7}else{0}}",
    "expected": 7,
    "full_source": "struct S{x:usize} fn get<'a>(p:&'a mut S)->&'a mut usize{&mut (*p).x} fn run()->i32{let mut p=S{x:3usize};let r=get(&mut p);*r=18446744073709551615usize;if p.x==18446744073709551615usize{7}else{0}}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "copy_aggregate_projection",
    "source": "struct S{p:(i32,i32)} fn get<'a>(s:&'a mut S)->&'a mut(i32,i32){&mut (*s).p} fn run()->i32{let mut s=S{p:(3,4)};let r=get(&mut s);*r=(7,8);s.p.0+s.p.1}",
    "expected": 15,
    "full_source": "struct S{p:(i32,i32)} fn get<'a>(s:&'a mut S)->&'a mut(i32,i32){&mut (*s).p} fn run()->i32{let mut s=S{p:(3,4)};let r=get(&mut s);*r=(7,8);s.p.0+s.p.1}\nfn main() { if run() == 15 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "independent_short_dependency",
    "source": "fn get<'a,'b>(p:&'a(i32,i32),q:&'b bool)->&'a i32{&(*p).1} fn run()->i32{let p=(3,7);let r:&i32;{let q=true;r=get(&p,&q);}*r}",
    "expected": 7,
    "full_source": "fn get<'a,'b>(p:&'a(i32,i32),q:&'b bool)->&'a i32{&(*p).1} fn run()->i32{let p=(3,7);let r:&i32;{let q=true;r=get(&p,&q);}*r}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "parent_resumes_after_lastuse",
    "source": "fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let parent=&mut p;let r=get(parent);*r=7;(*parent).0=8;(*parent).0+(*parent).1}",
    "expected": 15,
    "full_source": "fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let parent=&mut p;let r=get(parent);*r=7;(*parent).0=8;(*parent).0+(*parent).1}\nfn main() { if run() == 15 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "positive",
    "name": "disjoint_actual_fields",
    "source": "struct S{x:(i32,i32),y:(i32,i32)} fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut s=S{x:(1,3),y:(2,4)};let a=get(&mut s.x);let b=get(&mut s.y);*a=7;*b=8;s.x.1+s.y.1}",
    "expected": 15,
    "full_source": "struct S{x:(i32,i32),y:(i32,i32)} fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut s=S{x:(1,3),y:(2,4)};let a=get(&mut s.x);let b=get(&mut s.y);*a=7;*b=8;s.x.1+s.y.1}\nfn main() { if run() == 15 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "negative",
    "name": "private_field_direct_access",
    "source": "mod m{pub struct S{x:i32} pub fn make()->S{S{x:7}}} fn run()->i32{let p=m::make();p.x}",
    "expected": "E0616",
    "full_source": "mod m{pub struct S{x:i32} pub fn make()->S{S{x:7}}} fn run()->i32{let p=m::make();p.x}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "distinct_nominal_same_layout",
    "source": "struct A{x:i32} struct B{x:i32} struct S{a:A} fn get<'a>(p:&'a S)->&'a B{&(*p).a} fn run()->i32{7}",
    "expected": "E0308",
    "full_source": "struct A{x:i32} struct B{x:i32} struct S{a:A} fn get<'a>(p:&'a S)->&'a B{&(*p).a} fn run()->i32{7}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "parent_write_while_result_live",
    "source": "fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let r=get(&mut p);p.0=7;*r}",
    "expected": "E0506",
    "full_source": "fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let r=get(&mut p);p.0=7;*r}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "array_indices_conservatively_alias",
    "source": "fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut a=[(1,3),(2,4)];let x=get(&mut a[0]);let y=get(&mut a[1]);*x+*y}",
    "expected": "E0499",
    "full_source": "fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut a=[(1,3),(2,4)];let x=get(&mut a[0]);let y=get(&mut a[1]);*x+*y}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "owned_local_projected_return",
    "source": "fn get<'a>(p:&'a(i32,i32))->&'a i32{let q=(3,7);&q.1} fn run()->i32{7}",
    "expected": "E0515",
    "full_source": "fn get<'a>(p:&'a(i32,i32))->&'a i32{let q=(3,7);&q.1} fn run()->i32{7}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "unused_invalid_projected_callee",
    "source": "fn bad<'a>()->&'a i32{let q=(3,7);&q.1} fn run()->i32{7}",
    "expected": "E0515",
    "full_source": "fn bad<'a>()->&'a i32{let q=(3,7);&q.1} fn run()->i32{7}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "same_region_different_pointee_dependency",
    "source": "fn get<'a>(p:&'a(i32,i32),q:&'a bool)->&'a i32{&(*p).1} fn run()->i32{let p=(3,7);let r:&i32;{let q=true;r=get(&p,&q);}*r}",
    "expected": "E0597",
    "full_source": "fn get<'a>(p:&'a(i32,i32),q:&'a bool)->&'a i32{&(*p).1} fn run()->i32{let p=(3,7);let r:&i32;{let q=true;r=get(&p,&q);}*r}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "missing_projected_outlives",
    "source": "fn get<'a,'b>(p:&'b(i32,i32))->&'a i32{&(*p).1} fn run()->i32{7}",
    "expected": null,
    "full_source": "fn get<'a,'b>(p:&'b(i32,i32))->&'a i32{&(*p).1} fn run()->i32{7}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "negative",
    "name": "same_address_conflicting_authority",
    "source": "fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let parent=&mut p;let r=get(parent);let q=&mut (*parent).1;*r+*q}",
    "expected": "E0499",
    "full_source": "fn get<'a>(p:&'a mut(i32,i32))->&'a mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let parent=&mut p;let r=get(parent);let q=&mut (*parent).1;*r+*q}\nfn main() { run(); }\n",
    "expected_stdout": ""
  },
  {
    "group": "legalunsupported",
    "name": "enum_payload_projection",
    "source": "enum E{A(i32),B} fn get<'a>(p:&'a E)->&'a i32{match p{E::A(v)=>v,E::B=>loop{}}} fn run()->i32{let p=E::A(7);*get(&p)}",
    "expected": 7,
    "full_source": "enum E{A(i32),B} fn get<'a>(p:&'a E)->&'a i32{match p{E::A(v)=>v,E::B=>loop{}}} fn run()->i32{let p=E::A(7);*get(&p)}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "dynamic_index_projection",
    "source": "fn get<'a>(p:&'a[i32;2],i:usize)->&'a i32{&(*p)[i]} fn run()->i32{let p=[3,7];*get(&p,1usize)}",
    "expected": 7,
    "full_source": "fn get<'a>(p:&'a[i32;2],i:usize)->&'a i32{&(*p)[i]} fn run()->i32{let p=[3,7];*get(&p,1usize)}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "borrow_computed_call_result",
    "source": "fn get<'a>(p:&'a(i32,i32))->&'a i32{&(*p).1} fn run()->i32{let p=(3,7);let r=&*get(&p);*r}",
    "expected": 7,
    "full_source": "fn get<'a>(p:&'a(i32,i32))->&'a i32{&(*p).1} fn run()->i32{let p=(3,7);let r=&*get(&p);*r}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "mutable_input_shared_projection",
    "source": "fn get<'a>(p:&'a mut(i32,i32))->&'a i32{&(*p).1} fn run()->i32{let mut p=(3,7);*get(&mut p)}",
    "expected": 7,
    "full_source": "fn get<'a>(p:&'a mut(i32,i32))->&'a i32{&(*p).1} fn run()->i32{let mut p=(3,7);*get(&mut p)}\nfn main() { if run() == 7 { println!(\"projected returns oracle ok\"); } }\n",
    "expected_stdout": "projected returns oracle ok\n"
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
            prefix="jr-projected-reference-returns-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "borrowing-projected-reference-returns-v1",
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
            raise SystemExit(f"{totals['failure']} projected-input reference-return discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
