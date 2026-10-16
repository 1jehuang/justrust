#!/usr/bin/env python3
"""Bounded payload-enum differential corpus, not full Rust parity.

Exact stock-classified source bytes and stdout mirror tests/payload_enums.rs.
Tuple/braced payload construction, matching, ownership and aggregate argument/
return ABI are tested only within the agreed slice. Legal unsupported syntax is
recorded as boundaries, not invalid Rust or parity. Reuse module_correctness
process deadlines, bounded captures and workers 1/8 check/build infrastructure.
Native binaries must be supplied, never built by this harness.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "316be4ae1038f6a512073939b0cf1ad7975306cc9627b006828a2ca1d2a9ea14"

# Authoritative bear stock oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "group": "positive",
    "name": "tuple_variants",
    "source": "enum E{A(i32,bool),B} fn run()->i32{match E::A(7,true){E::A(n,b)=>if b{n}else{0},E::B=>0}}",
    "expected": 7,
    "full_source": "enum E{A(i32,bool),B} fn run()->i32{match E::A(7,true){E::A(n,b)=>if b{n}else{0},E::B=>0}}\nfn main() { if run() == 7 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "braced_variants",
    "source": "enum E{A{x:i32,y:i32},B} fn run()->i32{match (E::A{y:7,x:3}){E::A{x:a,y:b}=>a*10+b,E::B=>0}}",
    "expected": 37,
    "full_source": "enum E{A{x:i32,y:i32},B} fn run()->i32{match (E::A{y:7,x:3}){E::A{x:a,y:b}=>a*10+b,E::B=>0}}\nfn main() { if run() == 37 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "unit_and_empty_forms",
    "source": "enum E{A,B(),C{}} fn score(e:E)->i32{match e{E::A=>1,E::B()=>2,E::C{}=>3}} fn run()->i32{score(E::A{})+score(E::B())+score(E::C{})}",
    "expected": 6,
    "full_source": "enum E{A,B(),C{}} fn score(e:E)->i32{match e{E::A=>1,E::B()=>2,E::C{}=>3}} fn run()->i32{score(E::A{})+score(E::B())+score(E::C{})}\nfn main() { if run() == 6 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "tuple_rest",
    "source": "enum E{A(i32,i32,i32,i32),B} fn run()->i32{match E::A(1,2,3,4){E::A(a,..,d)=>a*10+d,E::B=>0}}",
    "expected": 14,
    "full_source": "enum E{A(i32,i32,i32,i32),B} fn run()->i32{match E::A(1,2,3,4){E::A(a,..,d)=>a*10+d,E::B=>0}}\nfn main() { if run() == 14 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "braced_rest_shorthand",
    "source": "enum E{A{x:i32,y:i32,z:i32},B} fn run()->i32{match (E::A{x:3,y:7,z:9}){E::A{x, y:renamed,..}=>x*10+renamed,E::B=>0}}",
    "expected": 37,
    "full_source": "enum E{A{x:i32,y:i32,z:i32},B} fn run()->i32{match (E::A{x:3,y:7,z:9}){E::A{x, y:renamed,..}=>x*10+renamed,E::B=>0}}\nfn main() { if run() == 37 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "singlevariant_tuple_let",
    "source": "enum E{A(i32,i32)} fn run()->i32{let E::A(a,b)=E::A(3,7);a*10+b}",
    "expected": 37,
    "full_source": "enum E{A(i32,i32)} fn run()->i32{let E::A(a,b)=E::A(3,7);a*10+b}\nfn main() { if run() == 37 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "singlevariant_braced_let",
    "source": "enum E{A{x:i32,y:i32}} fn run()->i32{let E::A{x, y:mut b}=E::A{x:3,y:4};b=b+3;x*10+b}",
    "expected": 37,
    "full_source": "enum E{A{x:i32,y:i32}} fn run()->i32{let E::A{x, y:mut b}=E::A{x:3,y:4};b=b+3;x*10+b}\nfn main() { if run() == 37 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "nested_tuple_struct_payload",
    "source": "struct S{n:i32} enum E{A((i32,S)),B} fn run()->i32{match E::A((3,S{n:7})){E::A((a,S{n:b}))=>a*10+b,E::B=>0}}",
    "expected": 37,
    "full_source": "struct S{n:i32} enum E{A((i32,S)),B} fn run()->i32{match E::A((3,S{n:7})){E::A((a,S{n:b}))=>a*10+b,E::B=>0}}\nfn main() { if run() == 37 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "nested_enum_payload_value",
    "source": "enum I{A(i32),B} enum E{A(I),B} fn run()->i32{match E::A(I::A(7)){E::A(inner)=>match inner{I::A(n)=>n,I::B=>0},E::B=>0}}",
    "expected": 7,
    "full_source": "enum I{A(i32),B} enum E{A(I),B} fn run()->i32{match E::A(I::A(7)){E::A(inner)=>match inner{I::A(n)=>n,I::B=>0},E::B=>0}}\nfn main() { if run() == 7 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "copy_payload_repeated",
    "source": "enum E{A(i32),B} fn score(e:E)->i32{match e{E::A(n)=>n,E::B=>0}} fn run()->i32{let e=E::A(7);let n=match e{E::A(n)=>n,E::B=>0};n+score(e)}",
    "expected": 14,
    "full_source": "enum E{A(i32),B} fn score(e:E)->i32{match e{E::A(n)=>n,E::B=>0}} fn run()->i32{let e=E::A(7);let n=match e{E::A(n)=>n,E::B=>0};n+score(e)}\nfn main() { if run() == 14 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "singleton_disjoint_moves",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S)} fn run()->i32{let e=E::A(S{n:3},S{n:7});let E::A(a,_)=e;let E::A(_,b)=e;take(a)+take(b)}",
    "expected": 10,
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S)} fn run()->i32{let e=E::A(S{n:3},S{n:7});let E::A(a,_)=e;let E::A(_,b)=e;take(a)+take(b)}\nfn main() { if run() == 10 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "singleton_braced_disjoint_moves",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A{x:S,y:S}} fn run()->i32{let e=E::A{x:S{n:3},y:S{n:7}};let E::A{x:a,..}=e;let E::A{y:b,..}=e;take(a)+take(b)}",
    "expected": 10,
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A{x:S,y:S}} fn run()->i32{let e=E::A{x:S{n:3},y:S{n:7}};let E::A{x:a,..}=e;let E::A{y:b,..}=e;take(a)+take(b)}\nfn main() { if run() == 10 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "selected_payload_fields",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S),B} fn run()->i32{match E::A(S{n:3},S{n:7}){E::A(a,b)=>take(a)+take(b),E::B=>0}}",
    "expected": 10,
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S),B} fn run()->i32{match E::A(S{n:3},S{n:7}){E::A(a,b)=>take(a)+take(b),E::B=>0}}\nfn main() { if run() == 10 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "wildcard_after_whole_move",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S),B} fn consume(e:E)->i32{match e{E::A(a)=>take(a),E::B=>0}} fn run()->i32{let e=E::A(S{n:3});let a=consume(e);a+match e{_=>7}}",
    "expected": 10,
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S),B} fn consume(e:E)->i32{match e{E::A(a)=>take(a),E::B=>0}} fn run()->i32{let e=E::A(S{n:3});let a=consume(e);a+match e{_=>7}}\nfn main() { if run() == 10 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "singleton_payload_wildcard_after_move",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S)} fn consume(e:E)->i32{let E::A(a)=e;take(a)} fn run()->i32{let e=E::A(S{n:3});let a=consume(e);a+match e{E::A(_)=>7}}",
    "expected": 10,
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S)} fn consume(e:E)->i32{let E::A(a)=e;take(a)} fn run()->i32{let e=E::A(S{n:3});let a=consume(e);a+match e{E::A(_)=>7}}\nfn main() { if run() == 10 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "whole_binding_payload",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S),B} fn consume(e:E)->i32{match e{E::A(a)=>take(a),E::B=>0}} fn run()->i32{let e=E::A(S{n:7});match e{x=>consume(x)}}",
    "expected": 7,
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S),B} fn consume(e:E)->i32{match e{E::A(a)=>take(a),E::B=>0}} fn run()->i32{let e=E::A(S{n:7});match e{x=>consume(x)}}\nfn main() { if run() == 7 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "constructor_source_order",
    "source": "enum E{A{x:i32,y:i32}} fn first()->i32{println!(\"first\");3} fn second()->i32{println!(\"second\");7} fn run()->i32{let e=E::A{y:first(),x:second()};let E::A{x,y}=e;x*10+y}",
    "expected": "first\nsecond\n73",
    "full_source": "enum E{A{x:i32,y:i32}} fn first()->i32{println!(\"first\");3} fn second()->i32{println!(\"second\");7} fn run()->i32{let e=E::A{y:first(),x:second()};let E::A{x,y}=e;x*10+y}\nfn main() { if run() == 73 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "first\nsecond\npayload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "large_return_argument_abi",
    "source": "enum E{A(i32,i32,i32,i32,i32,i32,i32,i32),B} fn make()->E{E::A(1,2,3,4,5,6,7,8)} fn sum(e:E)->i32{match e{E::A(a,b,c,d,e,f,g,h)=>a+b+c+d+e+f+g+h,E::B=>0}} fn run()->i32{sum(make())}",
    "expected": 36,
    "full_source": "enum E{A(i32,i32,i32,i32,i32,i32,i32,i32),B} fn make()->E{E::A(1,2,3,4,5,6,7,8)} fn sum(e:E)->i32{match e{E::A(a,b,c,d,e,f,g,h)=>a+b+c+d+e+f+g+h,E::B=>0}} fn run()->i32{sum(make())}\nfn main() { if run() == 36 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "recursive_function_abi",
    "source": "enum E{A(i32,i32),B} fn recur(e:E,n:i32)->i32{if n==0{match e{E::A(a,b)=>a+b,E::B=>0}}else{recur(e,n-1)}} fn run()->i32{recur(E::A(3,7),4)}",
    "expected": 10,
    "full_source": "enum E{A(i32,i32),B} fn recur(e:E,n:i32)->i32{if n==0{match e{E::A(a,b)=>a+b,E::B=>0}}else{recur(e,n-1)}} fn run()->i32{recur(E::A(3,7),4)}\nfn main() { if run() == 10 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "nominal_alias_import",
    "source": "enum E{A(i32),B} use E as Alias; use E::A as Make; fn run()->i32{match Make(7){Alias::A(n)=>n,Alias::B=>0}}",
    "expected": 7,
    "full_source": "enum E{A(i32),B} use E as Alias; use E::A as Make; fn run()->i32{match Make(7){Alias::A(n)=>n,Alias::B=>0}}\nfn main() { if run() == 7 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "public_module_payload",
    "source": "mod m{pub enum E{A{n:i32},B} pub fn make()->E{E::A{n:7}}} fn run()->i32{match m::make(){m::E::A{n}=>n,m::E::B=>0}}",
    "expected": 7,
    "full_source": "mod m{pub enum E{A{n:i32},B} pub fn make()->E{E::A{n:7}}} fn run()->i32{match m::make(){m::E::A{n}=>n,m::E::B=>0}}\nfn main() { if run() == 7 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "zero_sized_payload",
    "source": "enum E{A((),bool),B} fn run()->i32{match E::A((),true){E::A((),b)=>if b{7}else{0},E::B=>0}}",
    "expected": 7,
    "full_source": "enum E{A((),bool),B} fn run()->i32{match E::A((),true){E::A((),b)=>if b{7}else{0},E::B=>0}}\nfn main() { if run() == 7 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "negative",
    "name": "partial_multivariant_rematch",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S),B} fn run()->i32{let e=E::A(S{n:3},S{n:7});let n=match e{E::A(a,_)=>take(a),E::B=>0};n+match e{E::A(_,b)=>take(b),E::B=>0}}",
    "expected": "E0382",
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S),B} fn run()->i32{let e=E::A(S{n:3},S{n:7});let n=match e{E::A(a,_)=>take(a),E::B=>0};n+match e{E::A(_,b)=>take(b),E::B=>0}}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "whole_after_partial",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S)} fn consume(e:E)->i32{0} fn run()->i32{let e=E::A(S{n:3},S{n:7});let E::A(a,_)=e;take(a)+consume(e)}",
    "expected": "E0382",
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S)} fn consume(e:E)->i32{0} fn run()->i32{let e=E::A(S{n:3},S{n:7});let E::A(a,_)=e;take(a)+consume(e)}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "same_payload_twice",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S)} fn run()->i32{let e=E::A(S{n:3});let E::A(a)=e;let E::A(b)=e;take(a)+take(b)}",
    "expected": "E0382",
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S)} fn run()->i32{let e=E::A(S{n:3});let E::A(a)=e;let E::A(b)=e;take(a)+take(b)}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "whole_binding_moves",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S),B} fn consume(e:E)->i32{0} fn run()->i32{let e=E::A(S{n:7});let a=match e{x=>consume(x)};a+consume(e)}",
    "expected": "E0382",
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S),B} fn consume(e:E)->i32{0} fn run()->i32{let e=E::A(S{n:7});let a=match e{x=>consume(x)};a+consume(e)}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "enum_scalar_payload_noncopy",
    "source": "enum E{A(i32),B} fn consume(e:E)->i32{0} fn run()->i32{let e=E::A(7);let a=consume(e);a+consume(e)}",
    "expected": "E0382",
    "full_source": "enum E{A(i32),B} fn consume(e:E)->i32{0} fn run()->i32{let e=E::A(7);let a=consume(e);a+consume(e)}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "wrong_payload_type",
    "source": "enum E{A(i32)} fn run()->i32{let e=E::A(true);0}",
    "expected": "E0308",
    "full_source": "enum E{A(i32)} fn run()->i32{let e=E::A(true);0}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "wrong_nominal_enum",
    "source": "enum E{A(i32)} enum F{A(i32)} fn consume(e:E)->i32{0} fn run()->i32{consume(F::A(7))}",
    "expected": "E0308",
    "full_source": "enum E{A(i32)} enum F{A(i32)} fn consume(e:E)->i32{0} fn run()->i32{consume(F::A(7))}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "tuple_ctor_arity",
    "source": "enum E{A(i32,i32)} fn run()->i32{let e=E::A(7);0}",
    "expected": "E0061",
    "full_source": "enum E{A(i32,i32)} fn run()->i32{let e=E::A(7);0}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "tuple_pattern_arity",
    "source": "enum E{A(i32,i32)} fn run()->i32{match E::A(3,7){E::A(a)=>a}}",
    "expected": "E0023",
    "full_source": "enum E{A(i32,i32)} fn run()->i32{match E::A(3,7){E::A(a)=>a}}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "missing_braced_field",
    "source": "enum E{A{x:i32,y:i32}} fn run()->i32{let e=E::A{x:3};0}",
    "expected": "E0063",
    "full_source": "enum E{A{x:i32,y:i32}} fn run()->i32{let e=E::A{x:3};0}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "unknown_braced_field",
    "source": "enum E{A{x:i32}} fn run()->i32{let e=E::A{x:3,y:7};0}",
    "expected": "E0559",
    "full_source": "enum E{A{x:i32}} fn run()->i32{let e=E::A{x:3,y:7};0}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "refutable_variant_let",
    "source": "enum E{A(i32),B} fn run()->i32{let E::A(n)=E::A(7);n}",
    "expected": "E0005",
    "full_source": "enum E{A(i32),B} fn run()->i32{let E::A(n)=E::A(7);n}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "private_enum_payload",
    "source": "mod m{enum E{A(i32)}} fn run()->i32{let e=m::E::A(7);0}",
    "expected": "E0603",
    "full_source": "mod m{enum E{A(i32)}} fn run()->i32{let e=m::E::A(7);0}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "direct_recursive_enum",
    "source": "enum E{A(E)} fn run()->i32{0}",
    "expected": "E0072",
    "full_source": "enum E{A(E)} fn run()->i32{0}\nfn main() { run(); }\n"
  },
  {
    "group": "negative",
    "name": "indirect_recursive_enum",
    "source": "struct S{e:E} enum E{A(S)} fn run()->i32{0}",
    "expected": "E0072",
    "full_source": "struct S{e:E} enum E{A(S)} fn run()->i32{0}\nfn main() { run(); }\n"
  },
  {
    "group": "legalunsupported",
    "name": "nested_refutable_payload",
    "source": "enum I{A,B} enum E{X(I)} fn run()->i32{match E::X(I::A){E::X(I::A)=>3,E::X(I::B)=>7}}",
    "expected": 3,
    "full_source": "enum I{A,B} enum E{X(I)} fn run()->i32{match E::X(I::A){E::X(I::A)=>3,E::X(I::B)=>7}}\nfn main() { if run() == 3 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "payload_guard",
    "source": "enum E{A(i32),B} fn run()->i32{match E::A(7){E::A(n) if n>0=>n,_=>0}}",
    "expected": 7,
    "full_source": "enum E{A(i32),B} fn run()->i32{match E::A(7){E::A(n) if n>0=>n,_=>0}}\nfn main() { if run() == 7 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "payload_or_pattern",
    "source": "enum E{A(i32),B(i32)} fn run()->i32{match E::A(7){E::A(n)|E::B(n)=>n}}",
    "expected": 7,
    "full_source": "enum E{A(i32),B(i32)} fn run()->i32{match E::A(7){E::A(n)|E::B(n)=>n}}\nfn main() { if run() == 7 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "payload_reference_binding",
    "source": "enum E{A(i32)} fn run()->i32{let e=E::A(7);match e{E::A(ref n)=>*n}}",
    "expected": 7,
    "full_source": "enum E{A(i32)} fn run()->i32{let e=E::A(7);match e{E::A(ref n)=>*n}}\nfn main() { if run() == 7 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "legalunsupported",
    "name": "payload_drop",
    "source": "struct S{n:i32} impl Drop for S{fn drop(&mut self){}} enum E{A(S)} fn run()->i32{let e=E::A(S{n:7});match e{E::A(s)=>s.n}}",
    "expected": 7,
    "full_source": "struct S{n:i32} impl Drop for S{fn drop(&mut self){}} enum E{A(S)} fn run()->i32{let e=E::A(S{n:7});match e{E::A(s)=>s.n}}\nfn main() { if run() == 7 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "payload_binding_snapshot",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S),B} fn run()->i32{let mut e=E::A(S{n:3},S{n:7});let n=match e{E::A(a,b)=>{e=E::B;take(a)+take(b)},E::B=>0};n+match e{E::A(_,_)=>100,E::B=>1}}",
    "expected": 11,
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S),B} fn run()->i32{let mut e=E::A(S{n:3},S{n:7});let n=match e{E::A(a,b)=>{e=E::B;take(a)+take(b)},E::B=>0};n+match e{E::A(_,_)=>100,E::B=>1}}\nfn main() { if run() == 11 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
  },
  {
    "group": "positive",
    "name": "nested_singleton_payload_pattern",
    "source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum Inner{Item(S,S)} enum Outer{Wrap(Inner),Empty} fn run()->i32{match Outer::Wrap(Inner::Item(S{n:3},S{n:7})){Outer::Wrap(Inner::Item(S{n:a},b))=>a+take(b),Outer::Empty=>0}}",
    "expected": 10,
    "full_source": "struct S{n:i32} fn take(s:S)->i32{s.n} enum Inner{Item(S,S)} enum Outer{Wrap(Inner),Empty} fn run()->i32{match Outer::Wrap(Inner::Item(S{n:3},S{n:7})){Outer::Wrap(Inner::Item(S{n:a},b))=>a+take(b),Outer::Empty=>0}}\nfn main() { if run() == 10 { println!(\"payload oracle ok\"); } }\n",
    "expected_stdout": "payload oracle ok\n"
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
    assert len(cases) == 44 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-payload-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "payload-enums-v1",
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
            raise SystemExit(f"{totals['failure']} payload-enum discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
