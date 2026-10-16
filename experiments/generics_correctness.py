#!/usr/bin/env python3
"""Bounded generic template/instance oracle, not full Rust compatibility.

Exact frozen dictionaries/source/stdout/statuses mirror tests/generics.rs.
Normal cases reuse shared Case/run_case/process and bounded workers1/8
BUILD/CHECK with a supplied native binary. Never build a native binary here.
The one source-bound resource_error has distinct receipts, not invalid Rust or
legal-unsupported: stock BUILD1 recursion diagnostic, metadata CHECK0, no runtime;
native BUILD/CHECK1 with generic+limit diagnostic and no published artifacts or
success receipt. Signals, timeout, truncation and compiler crashes never pass.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time

from module_correctness import Case, accepted, completed, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "b792c18770c85af3754d32ac6298a0a5277a868ce314448e212dc25f411c4d97"

# Authoritative frozen bear oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{id::<i32>(7)}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "scalar_identity",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{id::<i32>(7)}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "struct S{x:i32}fn id<T>(x:T)->T{x}fn run()->i32{let s=id::<S>(S{x:7});s.x}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "noncopy_identity",
    "source": "struct S{x:i32}fn id<T>(x:T)->T{x}fn run()->i32{let s=id::<S>(S{x:7});s.x}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "struct Wrap<T>{value:T}fn wrap<T>(x:T)->Wrap<T>{Wrap::<T>{value:x}}fn run()->i32{wrap::<i32>(7).value}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "scalar_wrapper",
    "source": "struct Wrap<T>{value:T}fn wrap<T>(x:T)->Wrap<T>{Wrap::<T>{value:x}}fn run()->i32{wrap::<i32>(7).value}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "struct S{x:i32}struct Wrap<T>{value:T}fn wrap<T>(x:T)->Wrap<T>{Wrap::<T>{value:x}}fn run()->i32{let w=wrap::<S>(S{x:7});w.value.x}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "noncopy_wrapper",
    "source": "struct S{x:i32}struct Wrap<T>{value:T}fn wrap<T>(x:T)->Wrap<T>{Wrap::<T>{value:x}}fn run()->i32{let w=wrap::<S>(S{x:7});w.value.x}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "enum E<T>{Unit,Tuple(T),Braced{value:T}}fn get<T>(e:E<T>,fallback:T)->T{match e{E::Unit=>fallback,E::Tuple(v)=>v,E::Braced{value}=>value}}fn run()->i32{let a=get::<i32>(E::<i32>::Unit,1);let b=get::<i32>(E::<i32>::Tuple(2),0);let c=get::<i32>(E::<i32>::Braced{value:4},0);a+b+c}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "enum_variant_shapes",
    "source": "enum E<T>{Unit,Tuple(T),Braced{value:T}}fn get<T>(e:E<T>,fallback:T)->T{match e{E::Unit=>fallback,E::Tuple(v)=>v,E::Braced{value}=>value}}fn run()->i32{let a=get::<i32>(E::<i32>::Unit,1);let b=get::<i32>(E::<i32>::Tuple(2),0);let c=get::<i32>(E::<i32>::Braced{value:4},0);a+b+c}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "enum E<T>{A((T,i32)),B}fn extract<T>(e:E<T>,other:T)->T{match e{E::A((v,_))=>v,E::B=>other}}fn run()->i32{extract::<i32>(E::<i32>::A((7,2)),0)}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "tuple_payload_match",
    "source": "enum E<T>{A((T,i32)),B}fn extract<T>(e:E<T>,other:T)->T{match e{E::A((v,_))=>v,E::B=>other}}fn run()->i32{extract::<i32>(E::<i32>::A((7,2)),0)}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "enum E<T>{A{value:T,n:i32},B}fn extract<T>(e:E<T>,other:T)->T{match e{E::A{value:v,..}=>v,E::B=>other}}fn run()->i32{extract::<i32>(E::<i32>::A{value:7,n:2},0)}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "braced_payload_match",
    "source": "enum E<T>{A{value:T,n:i32},B}fn extract<T>(e:E<T>,other:T)->T{match e{E::A{value:v,..}=>v,E::B=>other}}fn run()->i32{extract::<i32>(E::<i32>::A{value:7,n:2},0)}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "struct Pair<T>{a:T,b:T}fn split<T>(p:Pair<T>)->(T,T){let a=p.a;let b=p.b;(a,b)}fn run()->i32{let p=split::<i32>(Pair::<i32>{a:3,b:4});p.0+p.1}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "struct_disjoint_partial_moves",
    "source": "struct Pair<T>{a:T,b:T}fn split<T>(p:Pair<T>)->(T,T){let a=p.a;let b=p.b;(a,b)}fn run()->i32{let p=split::<i32>(Pair::<i32>{a:3,b:4});p.0+p.1}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "enum Pair<T>{A{a:T,b:T}}fn split<T>(p:Pair<T>)->(T,T){let Pair::A{a,..}=p;let Pair::A{b,..}=p;(a,b)}fn run()->i32{let p=split::<i32>(Pair::<i32>::A{a:3,b:4});p.0+p.1}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "singleton_enum_disjoint_moves",
    "source": "enum Pair<T>{A{a:T,b:T}}fn split<T>(p:Pair<T>)->(T,T){let Pair::A{a,..}=p;let Pair::A{b,..}=p;(a,b)}fn run()->i32{let p=split::<i32>(Pair::<i32>::A{a:3,b:4});p.0+p.1}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn forward<U>(x:U)->U{id::<U>(x)}fn run()->i32{forward::<i32>(7)}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "explicit_generic_forwarding",
    "source": "fn id<T>(x:T)->T{x}fn forward<U>(x:U)->U{id::<U>(x)}fn run()->i32{forward::<i32>(7)}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "fn rec<T>(x:T,n:i32)->T{if n==0{x}else{rec::<T>(x,n-1)}}fn run()->i32{rec::<i32>(7,4)}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "exact_self_recursion",
    "source": "fn rec<T>(x:T,n:i32)->T{if n==0{x}else{rec::<T>(x,n-1)}}fn run()->i32{rec::<i32>(7,4)}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "fn a<T>(x:T,n:i32)->T{if n==0{x}else{b::<T>(x,n-1)}}fn b<U>(x:U,n:i32)->U{if n==0{x}else{a::<U>(x,n-1)}}fn run()->i32{a::<i32>(7,4)}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "exact_mutual_recursion",
    "source": "fn a<T>(x:T,n:i32)->T{if n==0{x}else{b::<T>(x,n-1)}}fn b<U>(x:U,n:i32)->U{if n==0{x}else{a::<U>(x,n-1)}}fn run()->i32{a::<i32>(7,4)}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn run()->i32{let a=id::<Wrap<i32>>(Wrap::<i32>{value:7});let b=id::<Wrap<u64>>(Wrap::<u64>{value:18446744073709551615u64});if b.value==18446744073709551615u64{a.value}else{0}}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "multiple_nominal_specializations",
    "source": "struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn run()->i32{let a=id::<Wrap<i32>>(Wrap::<i32>{value:7});let b=id::<Wrap<u64>>(Wrap::<u64>{value:18446744073709551615u64});if b.value==18446744073709551615u64{a.value}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "struct A<T>{value:T}struct B<T>{value:T}fn run()->i32{let a=A::<i32>{value:3};let b=B::<i32>{value:4};a.value+b.value}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "distinct_nominal_generic_owners",
    "source": "struct A<T>{value:T}struct B<T>{value:T}fn run()->i32{let a=A::<i32>{value:3};let b=B::<i32>{value:4};a.value+b.value}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "struct Wrap<T>{value:T}enum E<T>{A(T)}fn id<T>(x:T)->T{x}fn run()->i32{let w=id::<Wrap<E<([i32;1],i32)>>>(Wrap::<E<([i32;1],i32)>>{value:E::<([i32;1],i32)>::A(([3],4))});match w.value{E::A((a,b))=>a[0]+b}}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "nested_reference_free_substitution",
    "source": "struct Wrap<T>{value:T}enum E<T>{A(T)}fn id<T>(x:T)->T{x}fn run()->i32{let w=id::<Wrap<E<([i32;1],i32)>>>(Wrap::<E<([i32;1],i32)>>{value:E::<([i32;1],i32)>::A(([3],4))});match w.value{E::A((a,b))=>a[0]+b}}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "fn unused<T>(x:T)->T{x}fn run()->i32{7}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "valid_unused_template",
    "source": "fn unused<T>(x:T)->T{x}fn run()->i32{7}"
  },
  {
    "expected": "E0382",
    "expected_stdout": "",
    "full_source": "fn bad<T>(x:T)->(T,T){(x,x)}fn run()->i32{7}\nfn main(){run();}",
    "group": "negative",
    "name": "unused_duplicate",
    "source": "fn bad<T>(x:T)->(T,T){(x,x)}fn run()->i32{7}"
  },
  {
    "expected": "E0382",
    "expected_stdout": "",
    "full_source": "fn bad<T>(x:T)->(T,T){(x,x)}fn run()->i32{let p=bad::<i32>(7);p.0}\nfn main(){run();}",
    "group": "negative",
    "name": "duplicate_with_copy_caller",
    "source": "fn bad<T>(x:T)->(T,T){(x,x)}fn run()->i32{let p=bad::<i32>(7);p.0}"
  },
  {
    "expected": "E0369",
    "expected_stdout": "",
    "full_source": "fn bad<T>(x:T)->T{x+1}fn run()->i32{bad::<i32>(7)}\nfn main(){run();}",
    "group": "negative",
    "name": "generic_operator_copy_caller",
    "source": "fn bad<T>(x:T)->T{x+1}fn run()->i32{bad::<i32>(7)}"
  },
  {
    "expected": "E0308",
    "expected_stdout": "",
    "full_source": "fn bad<T>(x:T)->T{7i32}fn run()->i32{7}\nfn main(){run();}",
    "group": "negative",
    "name": "generic_return_mismatch",
    "source": "fn bad<T>(x:T)->T{7i32}fn run()->i32{7}"
  },
  {
    "expected": "E0382",
    "expected_stdout": "",
    "full_source": "struct Pair<T>{a:T,b:T}fn bad<T>(p:Pair<T>)->(T,T){let a=p.a;(a,p.a)}fn run()->i32{7}\nfn main(){run();}",
    "group": "negative",
    "name": "generic_field_reuse",
    "source": "struct Pair<T>{a:T,b:T}fn bad<T>(p:Pair<T>)->(T,T){let a=p.a;(a,p.a)}fn run()->i32{7}"
  },
  {
    "expected": "E0382",
    "expected_stdout": "",
    "full_source": "struct Pair<T>{a:T,b:T}fn bad<T>(p:Pair<T>)->Pair<T>{let a=p.a;p}fn run()->i32{7}\nfn main(){run();}",
    "group": "negative",
    "name": "whole_after_generic_partial_move",
    "source": "struct Pair<T>{a:T,b:T}fn bad<T>(p:Pair<T>)->Pair<T>{let a=p.a;p}fn run()->i32{7}"
  },
  {
    "expected": "E0308",
    "expected_stdout": "",
    "full_source": "struct Wrap<T>{value:T}fn take(x:Wrap<i32>)->i32{x.value}fn run()->i32{take(Wrap::<u8>{value:7})}\nfn main(){run();}",
    "group": "negative",
    "name": "nominal_argument_mismatch",
    "source": "struct Wrap<T>{value:T}fn take(x:Wrap<i32>)->i32{x.value}fn run()->i32{take(Wrap::<u8>{value:7})}"
  },
  {
    "expected": "E0107",
    "expected_stdout": "",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{id::<i32,u8>(7)}\nfn main(){run();}",
    "group": "negative",
    "name": "wrong_generic_arity",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{id::<i32,u8>(7)}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "fn dup<T:Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let p=dup::<i32>(7);p.0}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "positive",
    "name": "copy_bound_trait_dependency",
    "source": "fn dup<T:Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let p=dup::<i32>(7);p.0}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let x=7;let r=id::<&i32>(&x);*r}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "legalunsupported",
    "name": "direct_reference_substitution",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let x=7;let r=id::<&i32>(&x);*r}"
  },
  {
    "expected": 7,
    "expected_stdout": "generic oracle ok\n",
    "full_source": "struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn run()->i32{let x=7;let w=id::<Wrap<&i32>>(Wrap::<&i32>{value:&x});*w.value}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "legalunsupported",
    "name": "hidden_reference_substitution",
    "source": "struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn run()->i32{let x=7;let w=id::<Wrap<&i32>>(Wrap::<&i32>{value:&x});*w.value}"
  },
  {
    "expected": "recursion_limit",
    "expected_stdout": "",
    "full_source": "fn grow<T>(x:T){grow::<(T,)>((x,));}fn run()->i32{grow::<i32>(1);7}\nfn main(){if run()==7{println!(\"generic oracle ok\");}}",
    "group": "resource_error",
    "name": "expanding_recursive_instances",
    "source": "fn grow<T>(x:T){grow::<(T,)>((x,));}fn run()->i32{grow::<i32>(1);7}",
    "stock_check_status": 0,
    "stock_compile_status": 1,
    "stock_error_contains": "reached the recursion limit while instantiating",
    "stock_runtime_status": null
  }
]
''')


# Effective native-only promotions, backed by immutable worker1/8 BUILD/CHECK/run
# parity evidence. Frozen stock dictionaries and historical groups remain unchanged.
NATIVE_PROMOTIONS = {
    "direct_reference_substitution": "599626517ce80a4c43b98aebc1eec29d9b9ae95b5acea64c2d41a1a2c1b246d0",
}

def effective_native_case(case):
    expected_hash = NATIVE_PROMOTIONS.get(case.name)
    if expected_hash is None:
        return case
    if (case.kind != "boundary"
            or hashlib.sha256(case.source.encode()).hexdigest() != expected_hash):
        raise ValueError(f"native promotion changed its frozen source/classification: {case.name}")
    return Case(case.name, "valid", case.source, case.stdout)


def corpus():
    kinds = {"positive": "valid", "negative": "invalid",
             "legalunsupported": "boundary", "resource_error": "resource_error"}
    for fixture in FIXTURES:
        yield Case(fixture["name"], kinds[fixture["group"]],
                   fixture["full_source"], fixture.get("expected_stdout", ""))


RESOURCE_POLICY = (
    "exact source-bound resource_error only: stock BUILD1 with declared recursion "
    "diagnostic, CHECK0 with metadata, runtime null; native workers1/8 BUILD/CHECK1 "
    "with case-insensitive generic+limit, no parser/panic diagnostic, stdout success "
    "receipt or published native files; signals/timeouts/truncation/crashes fail")
RESOURCE_CASES = {f["name"]: f for f in FIXTURES if f["group"] == "resource_error"}
FIXTURE_BY_NAME = {f["name"]: f for f in FIXTURES}
assert set(RESOURCE_CASES) == {"expanding_recursive_instances"}
assert all(f["stock_compile_status"] == 1 and f["stock_check_status"] == 0
           and f["stock_runtime_status"] is None and f["expected"] == "recursion_limit"
           and f["stock_error_contains"] and f["expected_stdout"] == ""
           for f in RESOURCE_CASES.values())


def resource_fixture(case):
    fixture = RESOURCE_CASES.get(case.name)
    if (fixture is None or case.kind != "resource_error"
            or case.source != fixture["full_source"]
            or case.stdout != fixture["expected_stdout"]):
        return None
    return fixture


def resource_result(case, result, *, stock):
    """Only declared resource failures qualify, never arbitrary compiler failure."""
    fixture = resource_fixture(case)
    if fixture is None or not rejected(result) or result["stdout"]:
        return False
    if stock:
        return fixture["stock_error_contains"] in result["stderr"]
    diagnostic = result["stderr"].lower()
    return ("generic" in diagnostic and "limit" in diagnostic
            and not any(marker in diagnostic for marker in
                        ("panicked at", "parse error", "unexpected token", "unsupported syntax")))


def stock_resource_case(case, index, directory, rustc, compile_timeout, run_timeout):
    fixture = resource_fixture(case)
    if fixture is None:
        raise ValueError("resource case must match its exact frozen declaration")
    directory.mkdir()
    source = directory / "input.rs"
    source.write_text(case.source, encoding="utf-8")
    output = directory / "stock"
    metadata = directory / "stock.rmeta"
    assert not output.exists() and not metadata.exists()
    argv = [rustc, "--edition=2024", "--crate-name", "reference", "-C",
            "overflow-checks=yes", "-D", "overflowing_literals", "-D", "arithmetic_overflow", source]
    row = {"record": "stock_case", "index": index, "name": case.name,
           "kind": case.kind, "source": case.source,
           "source_sha256": hashlib.sha256(case.source.encode()).hexdigest(),
           "stock_compile": process([*argv, "-o", output], directory, compile_timeout),
           "expected_stock_compile_status": fixture["stock_compile_status"],
           "expected_stock_check_status": fixture["stock_check_status"],
           "expected_stock_runtime_status": fixture["stock_runtime_status"],
           "stock_run": None, "runtime_policy": RESOURCE_POLICY, "failures": []}
    row["stock_output_exists"] = output.exists()
    if not resource_result(case, row["stock_compile"], stock=True) or output.exists():
        row["failures"].append("stock resource BUILD must reject1 with exact recursion diagnostic and no executable")
    row["stock_check"] = process([*argv, "--emit=metadata", "-o", metadata],
                                  directory, compile_timeout)
    row["stock_metadata_exists"] = metadata.is_file()
    if not accepted(row["stock_check"]) or not metadata.is_file():
        row["failures"].append("stock resource CHECK must accept0 with metadata")
    row["passed"] = not row["failures"]
    row["outcome"] = "stock_resource_classified" if row["passed"] else "failure"
    return row


def run_resource_case(case, index, directory, binary, rustc, compile_timeout, run_timeout):
    start = time.perf_counter_ns()
    row = stock_resource_case(case, index, directory, rustc, compile_timeout, run_timeout)
    row["record"] = "case"
    row["workers"] = []
    source = directory / "input.rs"
    for workers in (1, 8):
        output = directory / f"native-{workers}"
        assert not output.exists()
        worker = {"workers": workers, "expected_compile_status": 1, "expected_check_status": 1,
                  "run": None}
        row["workers"].append(worker)
        for mode in ("check", "compile"):
            # Stock may leave a diagnostic long-type text file. Compare against
            # the post-stock baseline, never excuse a newly published native file.
            before = set(directory.iterdir())
            argv = [binary, source, "--workers", workers]
            argv += ["--check"] if mode == "check" else ["-o", output]
            result = process(argv, directory, compile_timeout)
            worker[mode] = result
            added = sorted(str(p.name) for p in set(directory.iterdir()) - before)
            worker[f"{mode}_published_files"] = added
            worker[f"{mode}_output_exists"] = output.exists()
            if not resource_result(case, result, stock=False) or output.exists() or added:
                row["failures"].append(f"native resource {mode} must reject1 generic+limit without publication with workers={workers}")
    row["passed"] = not row["failures"]
    row["outcome"] = "resource_rejected" if row["passed"] else "failure"
    row["wall_ns"] = time.perf_counter_ns() - start
    return row


def stock_metadata_phase(case, row, directory, rustc, compile_timeout):
    """Verify ordinary stock metadata without altering shared native semantics."""
    metadata = directory / "stock.rmeta"
    assert not metadata.exists()
    before = set(directory.iterdir())
    row["stock_check"] = process(
        [rustc, "--edition=2024", "--crate-name", "reference", "-C", "overflow-checks=yes",
         "-D", "overflowing_literals", "-D", "arithmetic_overflow", directory / "input.rs",
         "--emit=metadata", "-o", metadata], directory, compile_timeout)
    result = row["stock_check"]
    invalid = case.kind == "invalid"
    row["expected_stock_check_status"] = 1 if invalid else 0
    if invalid:
        good = rejected(result) and not metadata.exists() and set(directory.iterdir()) == before
        code = FIXTURE_BY_NAME[case.name]["expected"]
        if f"error[{code}]" not in row["stock_compile"]["stderr"]:
            row["failures"].append("stock BUILD diagnostic disagrees with frozen invalid code")
    else:
        good = accepted(result) and metadata.is_file() and set(directory.iterdir()) == before | {metadata}
    if not good:
        row["failures"].append("stock metadata status/artifact disagrees with frozen classification")
    row["passed"] = not row["failures"]
    outcome = ("stock_classified" if row["record"] == "stock_case" else
               "invalid_rejected" if invalid else
               "unsupported_boundary" if case.kind == "boundary" else "accepted_parity")
    row["outcome"] = outcome if row["passed"] else "failure"
    return row


def stock_case(case, index, directory, rustc, compile_timeout, run_timeout):
    if case.kind == "resource_error":
        return stock_resource_case(case, index, directory, rustc, compile_timeout, run_timeout)
    row = ordinary_stock_case(case, index, directory, rustc, compile_timeout, run_timeout)
    return stock_metadata_phase(case, row, directory, rustc, compile_timeout)


def native_case(case, index, directory, binary, rustc, compile_timeout, run_timeout):
    if case.kind == "resource_error":
        return run_resource_case(case, index, directory, binary, rustc, compile_timeout, run_timeout)
    original_kind = case.kind
    case = effective_native_case(case)
    row = run_case(case, index, directory, binary, rustc, compile_timeout, run_timeout)
    if case.name in NATIVE_PROMOTIONS:
        row["original_kind"] = original_kind
        row["native_promotion_source_sha256"] = NATIVE_PROMOTIONS[case.name]
    return stock_metadata_phase(case, row, directory, rustc, compile_timeout)


def ordinary_stock_case(case, index, directory, rustc, compile_timeout, run_timeout):
    """Classify independently without treating any native result as the oracle."""
    directory.mkdir()
    source = directory / "input.rs"
    source.write_text(case.source, encoding="utf-8")
    output = directory / "stock"
    result = process([rustc, "--edition=2024", "--crate-name", "reference", "-C",
                      "overflow-checks=yes", "-D", "overflowing_literals", "-D", "arithmetic_overflow", source, "-o", output], directory, compile_timeout)
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
    assert len(cases) == 28 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-generics-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "generics-v1",
                "case_count": len(cases),
                "case_counts": {kind: sum(c.kind == kind for c in cases)
                                for kind in ("valid", "invalid", "boundary", "resource_error")},
                "native_promotions": {} if args.stock_only else NATIVE_PROMOTIONS,
                "effective_native_case_counts": {} if args.stock_only else {
                    kind: sum(effective_native_case(c).kind == kind for c in cases)
                    for kind in ("valid", "invalid", "boundary", "resource_error")},
                "stock_only": args.stock_only,
                "workers": [] if args.stock_only else [1, 8],
                "modes": [] if args.stock_only else ["build", "check"], "edition": 2024,
                "reference_modes": ["build", "check", "run_if_stock_valid"], "rustc_path": str(rustc),
                "reference": version["stdout"],
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest() if binary else None,
                "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                "shared_harness_sha256": hashlib.sha256(Path(__file__).with_name("module_correctness.py").read_bytes()).hexdigest(),
                "corpus_sha256": corpus_hash,
                "acceptance_corpus_sha256": ACCEPTANCE_CORPUS_SHA256,
                "resource_policy": RESOURCE_POLICY,
                "timing_scope": "process/deadline/capture overhead included, not a compiler latency benchmark",
                "note": "legal unsupported boundaries are not invalid Rust or parity"})
        totals = {"accepted_parity": 0, "invalid_rejected": 0, "unsupported_boundary": 0,
                  "stock_classified": 0, "stock_resource_classified": 0,
                  "resource_rejected": 0, "failure": 0}
        for index, case in enumerate(cases):
            if args.stock_only:
                row = stock_case(case, index, directory / str(index), rustc,
                                 args.compile_timeout, args.run_timeout)
            else:
                row = native_case(case, index, directory / str(index), binary, rustc,
                               args.compile_timeout, args.run_timeout)
            row["expected_stdout"] = case.stdout
            record(row)
            totals[row["outcome"]] += 1
            print(f"{case.name}: {row['outcome']}", flush=True)
        record({"record": "summary", **totals, "passed": totals["failure"] == 0})
        if totals["failure"]:
            raise SystemExit(f"{totals['failure']} generics discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
