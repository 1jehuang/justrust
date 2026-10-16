#!/usr/bin/env python3
"""Bounded fieldless enum/match differential corpus, not full Rust parity.

Exact frozen stock-classified sources mirror tests/match.rs. Fieldless nonempty
nominal non-Copy enums and the approved finite/literal match slice are supported.
Payloads, derives, discriminants and other legal unsupported syntax are boundaries.
Reuse module_correctness process deadlines, bounded captures and workers 1/8
build/check validation. Native binaries must be supplied, never built here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "1f124303d810c6b417d98de2db98e862589bcfc12a800bb9128d53a89dbd7112"

# Frozen bear stock oracle. full_source is authoritative, with no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "group": "positive",
    "name": "variants",
    "source": "enum E { A, B, C } fn run()->i32 { let e=E::B; match e { E::A=>1, E::B=>2, E::C=>3 } }",
    "expected": 2,
    "full_source": "enum E { A, B, C } fn run()->i32 { let e=E::B; match e { E::A=>1, E::B=>2, E::C=>3 } }\nfn main() { if run() == 2 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "repeat_noncopy",
    "source": "enum E { A, B, C } fn run()->i32 { let e=E::B; let a=match e { E::A=>1, _=>2 }; let b=match e { _=>3 }; a+b+match e { E::A=>4,E::B=>5,E::C=>6 } }",
    "expected": 10,
    "full_source": "enum E { A, B, C } fn run()->i32 { let e=E::B; let a=match e { E::A=>1, _=>2 }; let b=match e { _=>3 }; a+b+match e { E::A=>4,E::B=>5,E::C=>6 } }\nfn main() { if run() == 10 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "binding_moves",
    "source": "enum E { A, B, C } fn score(e:E)->i32 { match e { E::A=>1,_=>2 } } fn run()->i32 { let e=E::B; match e { x=>score(x) } }",
    "expected": 2,
    "full_source": "enum E { A, B, C } fn score(e:E)->i32 { match e { E::A=>1,_=>2 } } fn run()->i32 { let e=E::B; match e { x=>score(x) } }\nfn main() { if run() == 2 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "pass_return",
    "source": "enum E { A, B, C } fn make()->E { E::C } fn score(e:E)->i32 { match e { E::A=>1,E::B=>2,E::C=>3 } } fn run()->i32 { score(make()) }",
    "expected": 3,
    "full_source": "enum E { A, B, C } fn make()->E { E::C } fn score(e:E)->i32 { match e { E::A=>1,E::B=>2,E::C=>3 } } fn run()->i32 { score(make()) }\nfn main() { if run() == 3 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "reinitialize",
    "source": "enum E { A, B, C } fn run()->i32 { let mut e=E::A; let x=match e { E::A=>1,_=>2 }; e=E::C; x+match e { E::C=>3,_=>4 } }",
    "expected": 4,
    "full_source": "enum E { A, B, C } fn run()->i32 { let mut e=E::A; let x=match e { E::A=>1,_=>2 }; e=E::C; x+match e { E::C=>3,_=>4 } }\nfn main() { if run() == 4 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "arm_blocks",
    "source": "enum E { A, B, C } fn run()->i32 { match E::B { E::A=>{let n=1;n}, E::B=>{let n=2;n+3}, E::C=>6 } }",
    "expected": 5,
    "full_source": "enum E { A, B, C } fn run()->i32 { match E::B { E::A=>{let n=1;n}, E::B=>{let n=2;n+3}, E::C=>6 } }\nfn main() { if run() == 5 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "nested",
    "source": "enum E { A, B, C } fn run()->i32 { match E::A { E::A=>match E::C { E::C=>7,_=>8 },_=>9 } }",
    "expected": 7,
    "full_source": "enum E { A, B, C } fn run()->i32 { match E::A { E::A=>match E::C { E::C=>7,_=>8 },_=>9 } }\nfn main() { if run() == 7 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "projection_nonconsume",
    "source": "enum E { A, B, C } struct Pair { e:E,n:i32 } fn run()->i32 { let p=Pair{e:E::B,n:4}; let x=match p.e { E::B=>2,_=>3 }; x+match p.e { _=>p.n } }",
    "expected": 6,
    "full_source": "enum E { A, B, C } struct Pair { e:E,n:i32 } fn run()->i32 { let p=Pair{e:E::B,n:4}; let x=match p.e { E::B=>2,_=>3 }; x+match p.e { _=>p.n } }\nfn main() { if run() == 6 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "negative",
    "name": "not_exhaustive",
    "source": "enum E { A, B, C } fn run()->i32 { match E::A { E::A=>1,E::B=>2 } }",
    "expected": "E0004",
    "full_source": "enum E { A, B, C } fn run()->i32 { match E::A { E::A=>1,E::B=>2 } }\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "arm_types",
    "source": "enum E { A, B, C } fn run()->i32 { match E::A { E::A=>1,_=>true } }",
    "expected": "E0308",
    "full_source": "enum E { A, B, C } fn run()->i32 { match E::A { E::A=>1,_=>true } }\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "wrong_enum",
    "source": "enum E { A, B, C } enum F { A } fn run()->i32 { match E::A { F::A=>1,_=>2 } }",
    "expected": "E0308",
    "full_source": "enum E { A, B, C } enum F { A } fn run()->i32 { match E::A { F::A=>1,_=>2 } }\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "unknown_variant",
    "source": "enum E { A, B, C } fn run()->i32 { match E::A { E::Missing=>1,_=>2 } }",
    "expected": "E0599",
    "full_source": "enum E { A, B, C } fn run()->i32 { match E::A { E::Missing=>1,_=>2 } }\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "binding_use_after_move",
    "source": "enum E { A, B, C } fn take(e:E)->i32 { 1 } fn run()->i32 { let e=E::A; let n=match e { x=>take(x) }; n+take(e) }",
    "expected": "E0382",
    "full_source": "enum E { A, B, C } fn take(e:E)->i32 { 1 } fn run()->i32 { let e=E::A; let n=match e { x=>take(x) }; n+take(e) }\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "plain_move",
    "source": "enum E { A, B, C } fn take(e:E)->i32 { 1 } fn run()->i32 { let e=E::A; let x=e; take(x)+take(e) }",
    "expected": "E0382",
    "full_source": "enum E { A, B, C } fn take(e:E)->i32 { 1 } fn run()->i32 { let e=E::A; let x=e; take(x)+take(e) }\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "arm_scope",
    "source": "enum E { A, B, C } fn run()->i32 { let n=match E::A { x=>1 }; n+match x { _=>2 } }",
    "expected": "E0425",
    "full_source": "enum E { A, B, C } fn run()->i32 { let n=match E::A { x=>1 }; n+match x { _=>2 } }\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "duplicate_variant",
    "source": "enum E { A,A } fn run()->i32 { 1 }",
    "expected": "E0428",
    "full_source": "enum E { A,A } fn run()->i32 { 1 }\nfn main() { run(); }\n"
  },
  {
    "group": "positive",
    "name": "payload",
    "source": "enum E { A(i32),B } fn run()->i32 { match E::A(4) { E::A(n)=>n,E::B=>0 } }",
    "expected": 4,
    "full_source": "enum E { A(i32),B } fn run()->i32 { match E::A(4) { E::A(n)=>n,E::B=>0 } }\nfn main() { if run() == 4 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "wildcard_moved",
    "source": "enum E { A,B } fn take(e:E)->i32{1} fn run()->i32{let e=E::A; let n=take(e); n+match e {_=>2}}",
    "expected": 3,
    "full_source": "enum E { A,B } fn take(e:E)->i32{1} fn run()->i32{let e=E::A; let n=take(e); n+match e {_=>2}}\nfn main() { if run() == 3 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "guard",
    "source": "enum E { A, B, C } fn run()->i32 { match E::A { E::A if true=>1,_=>2 } }",
    "expected": 1,
    "full_source": "enum E { A, B, C } fn run()->i32 { match E::A { E::A if true=>1,_=>2 } }\nfn main() { if run() == 1 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "or_pattern",
    "source": "enum E { A, B, C } fn run()->i32 { match E::A { E::A|E::B=>1,E::C=>2 } }",
    "expected": 1,
    "full_source": "enum E { A, B, C } fn run()->i32 { match E::A { E::A|E::B=>1,E::C=>2 } }\nfn main() { if run() == 1 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "ref_binding",
    "source": "enum E { A, B, C } fn run()->i32 { let e=E::A; match e { ref x=>1 } }",
    "expected": 1,
    "full_source": "enum E { A, B, C } fn run()->i32 { let e=E::A; match e { ref x=>1 } }\nfn main() { if run() == 1 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "at_binding",
    "source": "enum E { A, B, C } fn run()->i32 { match E::A { x @ E::A=>1,_=>2 } }",
    "expected": 1,
    "full_source": "enum E { A, B, C } fn run()->i32 { match E::A { x @ E::A=>1,_=>2 } }\nfn main() { if run() == 1 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "explicit_discriminant",
    "source": "enum E { A=5,B=9 } fn run()->i32 { match E::B { E::A=>1,E::B=>2 } }",
    "expected": 2,
    "full_source": "enum E { A=5,B=9 } fn run()->i32 { match E::B { E::A=>1,E::B=>2 } }\nfn main() { if run() == 2 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "unreachable_arm",
    "source": "enum E { A, B, C } fn run()->i32 { match E::A { _=>1,E::A=>2 } }",
    "expected": 1,
    "full_source": "enum E { A, B, C } fn run()->i32 { match E::A { _=>1,E::A=>2 } }\nfn main() { if run() == 1 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "bool_coverage",
    "source": "fn run()->i32{match true{true=>4,false=>5}}",
    "expected": 4,
    "full_source": "fn run()->i32{match true{true=>4,false=>5}}\nfn main() { if run() == 4 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "unit_coverage",
    "source": "fn run()->i32{match (){()=>6}}",
    "expected": 6,
    "full_source": "fn run()->i32{match (){()=>6}}\nfn main() { if run() == 6 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "integer_catchall",
    "source": "fn score(x:i32)->i32{match x{0=>7,9=>8,_=>3}} fn run()->i32{score(0)+score(9)+score(-1)}",
    "expected": 18,
    "full_source": "fn score(x:i32)->i32{match x{0=>7,9=>8,_=>3}} fn run()->i32{score(0)+score(9)+score(-1)}\nfn main() { if run() == 18 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "shadow_binding",
    "source": "fn run()->i32{let n=7; let x=match 4{n=>n+1}; x+n}",
    "expected": 12,
    "full_source": "fn run()->i32{let n=7; let x=match 4{n=>n+1}; x+n}\nfn main() { if run() == 12 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "mutable_binding",
    "source": "fn run()->i32{match 4{mut n=>{n=n+3;n}}}",
    "expected": 7,
    "full_source": "fn run()->i32{match 4{mut n=>{n=n+3;n}}}\nfn main() { if run() == 7 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "tuple_result",
    "source": "fn run()->i32{let p=match true{true=>(3,4),false=>(5,6)}; p.0*10+p.1}",
    "expected": 34,
    "full_source": "fn run()->i32{let p=match true{true=>(3,4),false=>(5,6)}; p.0*10+p.1}\nfn main() { if run() == 34 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "struct_result",
    "source": "struct S{n:i32} fn run()->i32{let s=match false{true=>S{n:3},false=>S{n:8}};s.n}",
    "expected": 8,
    "full_source": "struct S{n:i32} fn run()->i32{let s=match false{true=>S{n:3},false=>S{n:8}};s.n}\nfn main() { if run() == 8 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "enum_result",
    "source": "enum E { A, B, C } fn run()->i32{let e=match true{true=>E::B,false=>E::C};match e{E::A=>1,E::B=>2,E::C=>3}}",
    "expected": 2,
    "full_source": "enum E { A, B, C } fn run()->i32{let e=match true{true=>E::B,false=>E::C};match e{E::A=>1,E::B=>2,E::C=>3}}\nfn main() { if run() == 2 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "wildcard_uninitialized",
    "source": "enum E { A, B, C } fn run()->i32{let e:E;match e{_=>7}}",
    "expected": 7,
    "full_source": "enum E { A, B, C } fn run()->i32{let e:E;match e{_=>7}}\nfn main() { if run() == 7 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "scrutinee_once",
    "source": "enum E { A, B, C } fn make()->E{println!(\"once\");E::B} fn run()->i32{match make(){E::A=>1,E::B=>2,E::C=>3}}",
    "expected": "once\n2",
    "full_source": "enum E { A, B, C } fn make()->E{println!(\"once\");E::B} fn run()->i32{match make(){E::A=>1,E::B=>2,E::C=>3}}\nfn main() { if run() == 2 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "once\nmatch oracle ok\n"
  },
  {
    "group": "positive",
    "name": "wildcard_effect_once",
    "source": "fn make()->i32{println!(\"once\");4} fn run()->i32{match make(){_=>7}}",
    "expected": "once\n7",
    "full_source": "fn make()->i32{println!(\"once\");4} fn run()->i32{match make(){_=>7}}\nfn main() { if run() == 7 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "once\nmatch oracle ok\n"
  },
  {
    "group": "positive",
    "name": "duplicate_arm_first",
    "source": "enum E { A, B, C } fn run()->i32{match E::A{E::A=>3,E::A=>4,_=>5}}",
    "expected": 3,
    "full_source": "enum E { A, B, C } fn run()->i32{match E::A{E::A=>3,E::A=>4,_=>5}}\nfn main() { if run() == 3 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "grouped_import",
    "source": "enum E { A, B, C } use E::{A,B,C}; fn run()->i32{match B{A=>1,B=>2,C=>3}}",
    "expected": 2,
    "full_source": "enum E { A, B, C } use E::{A,B,C}; fn run()->i32{match B{A=>1,B=>2,C=>3}}\nfn main() { if run() == 2 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "type_alias",
    "source": "enum E { A, B, C } type Alias=E; fn run()->i32{match Alias::B{Alias::A=>1,Alias::B=>2,Alias::C=>3}}",
    "expected": 2,
    "full_source": "enum E { A, B, C } type Alias=E; fn run()->i32{match Alias::B{Alias::A=>1,Alias::B=>2,Alias::C=>3}}\nfn main() { if run() == 2 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "bare_binder",
    "source": "enum E{A,B} fn score(e:E)->i32{match e{E::A=>1,E::B=>2}} fn run()->i32{match E::B{e=>score(e)}}",
    "expected": 2,
    "full_source": "enum E{A,B} fn score(e:E)->i32{match e{E::A=>1,E::B=>2}} fn run()->i32{match E::B{e=>score(e)}}\nfn main() { if run() == 2 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "negative",
    "name": "variant_after_move",
    "source": "enum E { A, B, C } fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=take(e);n+match e{E::A=>1,_=>2}}",
    "expected": "E0382",
    "full_source": "enum E { A, B, C } fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=take(e);n+match e{E::A=>1,_=>2}}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "variant_uninitialized",
    "source": "enum E { A, B, C } fn run()->i32{let e:E;match e{E::A=>1,_=>2}}",
    "expected": "E0381",
    "full_source": "enum E { A, B, C } fn run()->i32{let e:E;match e{E::A=>1,_=>2}}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "arm_join_move",
    "source": "enum E { A, B, C } fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=match true{true=>take(e),false=>0};n+take(e)}",
    "expected": "E0382",
    "full_source": "enum E { A, B, C } fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=match true{true=>take(e),false=>0};n+take(e)}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "unreachable_still_typed",
    "source": "enum E { A, B, C } fn run()->i32{match E::A{_=>1,E::A=>false}}",
    "expected": "E0308",
    "full_source": "enum E { A, B, C } fn run()->i32{match E::A{_=>1,E::A=>false}}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "integer_no_catchall",
    "source": "fn run()->i32{match 1{1=>2}}",
    "expected": "E0004",
    "full_source": "fn run()->i32{match 1{1=>2}}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "bool_not_exhaustive",
    "source": "fn run()->i32{match true{true=>2}}",
    "expected": "E0004",
    "full_source": "fn run()->i32{match true{true=>2}}\nfn main() { run(); }\n"
  },
  {
    "group": "legalunsupported",
    "name": "empty_enum",
    "source": "enum E{} fn run()->i32{3}",
    "expected": 3,
    "full_source": "enum E{} fn run()->i32{3}\nfn main() { if run() == 3 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "generic_enum",
    "source": "enum E<T>{A(T)} fn run()->i32{match E::A(3){E::A(n)=>n}}",
    "expected": 3,
    "full_source": "enum E<T>{A(T)} fn run()->i32{match E::A(3){E::A(n)=>n}}\nfn main() { if run() == 3 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "range_pattern",
    "source": "fn run()->i32{match 2{1..=3=>4,_=>5}}",
    "expected": 4,
    "full_source": "fn run()->i32{match 2{1..=3=>4,_=>5}}\nfn main() { if run() == 4 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "nested_refutable",
    "source": "enum E{A(bool)} fn run()->i32{match E::A(true){E::A(true)=>1,E::A(false)=>2}}",
    "expected": 1,
    "full_source": "enum E{A(bool)} fn run()->i32{match E::A(true){E::A(true)=>1,E::A(false)=>2}}\nfn main() { if run() == 1 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "module_namespace",
    "source": "mod m { pub enum E{A,B} pub fn make()->E{E::B} } fn run()->i32{match m::make(){m::E::A=>1,m::E::B=>2}}",
    "expected": 2,
    "full_source": "mod m { pub enum E{A,B} pub fn make()->E{E::B} } fn run()->i32{match m::make(){m::E::A=>1,m::E::B=>2}}\nfn main() { if run() == 2 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "negative",
    "name": "private_enum",
    "source": "mod m { enum E{A} } fn run()->i32{match m::E::A{_=>1}}",
    "expected": "E0603",
    "full_source": "mod m { enum E{A} } fn run()->i32{match m::E::A{_=>1}}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "enum_not_value",
    "source": "enum E { A, B, C } fn run()->i32{let x=E;1}",
    "expected": "E0423",
    "full_source": "enum E { A, B, C } fn run()->i32{let x=E;1}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "dead_move_join",
    "source": "enum E { A, B, C } fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=match true{_=>0,true=>take(e)};n+take(e)}",
    "expected": "E0382",
    "full_source": "enum E { A, B, C } fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=match true{_=>0,true=>take(e)};n+take(e)}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "moved_wildcard_then_variant",
    "source": "enum E { A, B, C } fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=take(e);n+match e{_=>2,E::A=>3}}",
    "expected": "E0382",
    "full_source": "enum E { A, B, C } fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=take(e);n+match e{_=>2,E::A=>3}}\nfn main() { run(); }\n"
  },
  {
    "group": "positive",
    "name": "singleton_after_move",
    "source": "enum E{A} fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=take(e);n+match e{E::A=>2}}",
    "expected": 3,
    "full_source": "enum E{A} fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=take(e);n+match e{E::A=>2}}\nfn main() { if run() == 3 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "negative",
    "name": "dead_break_type",
    "source": "fn run()->i32{loop{match true{_=>break 1,true=>break false}}}",
    "expected": "E0308",
    "full_source": "fn run()->i32{loop{match true{_=>break 1,true=>break false}}}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "imported_variant_parameter",
    "source": "enum E { A, B, C } use E::A; fn score(A:E)->i32{1} fn run()->i32{score(E::A)}",
    "expected": "E0005",
    "full_source": "enum E { A, B, C } use E::A; fn score(A:E)->i32{1} fn run()->i32{score(E::A)}\nfn main() { run(); }\n"
  },
  {
    "group": "positive",
    "name": "imported_variant",
    "source": "enum E { A, B, C } use E::A; use E::B; use E::C; fn run()->i32{match B{A=>1,B=>2,C=>3}}",
    "expected": 2,
    "full_source": "enum E { A, B, C } use E::A; use E::B; use E::C; fn run()->i32{match B{A=>1,B=>2,C=>3}}\nfn main() { if run() == 2 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
  },
  {
    "group": "positive",
    "name": "alias_variant",
    "source": "enum E { A, B, C } use E as Alias; fn run()->i32{match Alias::B{Alias::A=>1,Alias::B=>2,Alias::C=>3}}",
    "expected": 2,
    "full_source": "enum E { A, B, C } use E as Alias; fn run()->i32{match Alias::B{Alias::A=>1,Alias::B=>2,Alias::C=>3}}\nfn main() { if run() == 2 { println!(\"match oracle ok\"); } }\n",
    "expected_stdout": "match oracle ok\n"
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
    assert len(cases) == 59 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-match-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "match-fieldless-v1",
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
            raise SystemExit(f"{totals['failure']} match discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
