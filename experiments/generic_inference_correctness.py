#!/usr/bin/env python3
"""Bounded first-order generic-inference differential corpus, not full Rust.

Exact frozen stock-classified 28 dictionaries and full_source/stdout bytes.
Only current-callee inference is claimed, with four legal excluded boundaries.
Reuse shared bounded Case/run_case/process for supplied-native workers1/8
BUILD/CHECK. Never build a native compiler here. Stock runtime at the
unconstrained local site does not independently prove its fallback type.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "fdd1a268685f4eba121b10d7e354b93ef7375fb7b0fa06639702e0a49c6df845"

# Authoritative frozen stock oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn same<T>(a:T,b:T)->T{b}fn run()->i32{let x=same(1,2u8);if x==2u8{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "later_known_integer_width",
    "proposed_group": "positive",
    "source": "fn same<T>(a:T,b:T)->T{b}fn run()->i32{let x=same(1,2u8);if x==2u8{7}else{0}}",
    "source_bytes": 135,
    "source_sha256": "caa56ddbf07fad24ebb81ebb04ce9b93bd308fb713aafd7e49bb36612de57d77",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let x:u8=id(7);if x==7u8{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "hard_result_identity_context",
    "proposed_group": "positive",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let x:u8=id(7);if x==7u8{7}else{0}}",
    "source_bytes": 126,
    "source_sha256": "0f3503cca5a5bf2d094299257a8eca374413ddcd2d7caf97300e740d3fc74ddf",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let x=id(7);let y=id(-((1)));let z=id(!0);if x==7 && y == -1 && z == -1{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "unsuffixed_identity_default",
    "proposed_group": "positive",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let x=id(7);let y=id(-((1)));let z=id(!0);if x==7 && y == -1 && z == -1{7}else{0}}",
    "source_bytes": 173,
    "source_sha256": "d293e54f808f224e0a951f2b2db0f2629d052edc5f4165a92f619502b1af3810",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let p=id((3u8,4u16));let a=id([3u8,4u8]);let r=id([7u16;1]);let empty:[u8;0]=id([]);if p.0==3u8 && p.1==4u16 && a[1]==4u8 && r[0]==7u16{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "tuple_array_structural_evidence",
    "proposed_group": "positive",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let p=id((3u8,4u16));let a=id([3u8,4u8]);let r=id([7u16;1]);let empty:[u8;0]=id([]);if p.0==3u8 && p.1==4u16 && a[1]==4u8 && r[0]==7u16{7}else{0}}",
    "source_bytes": 237,
    "source_sha256": "f1069b6ad3b62467775679f62a0a6f1cdded37e8ff29a5f9dd5e903485a94e25",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn known()->u8{7u8}fn run()->i32{let a=id(known());let b=id(id::<u16>(7u16));let c=id(7i32 as u64);let d=id(256) as u8;if a==7u8 && b==7u16 && c==7u64 && d==0u8{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "known_call_and_cast_evidence",
    "proposed_group": "positive",
    "source": "fn id<T>(x:T)->T{x}fn known()->u8{7u8}fn run()->i32{let a=id(known());let b=id(id::<u16>(7u16));let c=id(7i32 as u64);let d=id(256) as u8;if a==7u8 && b==7u16 && c==7u64 && d==0u8{7}else{0}}",
    "source_bytes": 248,
    "source_sha256": "9a5266305001fceab795744164fa61d488c87c213b627f3ffc8efd06249dc04b",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "struct Wrap<T>{value:T}fn run()->i32{let a=Wrap{value:3u8};let b:Wrap<u64>=Wrap{value:4};if a.value==3u8 && b.value==4u64{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "inferred_struct_constructors",
    "proposed_group": "positive",
    "source": "struct Wrap<T>{value:T}fn run()->i32{let a=Wrap{value:3u8};let b:Wrap<u64>=Wrap{value:4};if a.value==3u8 && b.value==4u64{7}else{0}}",
    "source_bytes": 190,
    "source_sha256": "6c9a19c34608def236e99a7da8c8d7804030198fa1d6210dda242de519feeec8",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "enum E<T>{None,Some(T),Named{value:T}}fn take<T>(e:E<T>,fallback:T)->T{match e{E::None=>fallback,E::Some(v)=>v,E::Named{value}=>value}}fn run()->i32{let a:E<u8>=E::None;let b=E::Some(3u8);let c=E::Named{value:4u8};if take(a,0u8)==0u8 && take(b,0u8)==3u8 && take(c,0u8)==4u8{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "inferred_enum_constructors",
    "proposed_group": "positive",
    "source": "enum E<T>{None,Some(T),Named{value:T}}fn take<T>(e:E<T>,fallback:T)->T{match e{E::None=>fallback,E::Some(v)=>v,E::Named{value}=>value}}fn run()->i32{let a:E<u8>=E::None;let b=E::Some(3u8);let c=E::Named{value:4u8};if take(a,0u8)==0u8 && take(b,0u8)==3u8 && take(c,0u8)==4u8{7}else{0}}",
    "source_bytes": 342,
    "source_sha256": "2559295cfab0f01268301181368904e625d82510c457a958a4ba85ea0304c817",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn forward<T>(x:T)->T{id(x)}fn run()->i32{forward(7)}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "rigid_generic_forwarding",
    "proposed_group": "positive",
    "source": "fn id<T>(x:T)->T{x}fn forward<T>(x:T)->T{id(x)}fn run()->i32{forward(7)}",
    "source_bytes": 130,
    "source_sha256": "33961baed957a6e5575bbc846ef586ad591dff254cc3b7fa9313e2afa7d77f69",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn rec<T>(x:T,n:i32)->T{if n==0{x}else{rec(x,n-1)}}fn run()->i32{let x=rec(7u8,4);if x==7u8{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "rigid_exact_self_recursion",
    "proposed_group": "positive",
    "source": "fn rec<T>(x:T,n:i32)->T{if n==0{x}else{rec(x,n-1)}}fn run()->i32{let x=rec(7u8,4);if x==7u8{7}else{0}}",
    "source_bytes": 160,
    "source_sha256": "8599a481f1a9a9fc4d69f2ac3e6a624a590efa58f85b054f4fd1c4320dfbaaad",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}struct S{x:i32}fn run()->i32{let s=id(S{x:7});s.x}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "noncopy_inferred_call_move",
    "proposed_group": "positive",
    "source": "fn id<T>(x:T)->T{x}struct S{x:i32}fn run()->i32{let s=id(S{x:7});s.x}",
    "source_bytes": 127,
    "source_sha256": "e7d4d1b2ca8c58bf210db2551b421955c1eeb1c07b720cef649d44be13a5c515",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "struct S{x:i32}struct Pair<T>{a:T,b:T}fn run()->i32{let p=Pair{a:S{x:3},b:S{x:4}};let a=p.a;let b=p.b;a.x+b.x}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "constructor_disjoint_partial_moves",
    "proposed_group": "positive",
    "source": "struct S{x:i32}struct Pair<T>{a:T,b:T}fn run()->i32{let p=Pair{a:S{x:3},b:S{x:4}};let a=p.a;let b=p.b;a.x+b.x}",
    "source_bytes": 168,
    "source_sha256": "e1b422cf5f4d93fecd308c16dca03208636a4fc340dbaff872f719db5bfc7092",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn same<T>(a:T,b:T)->T{b}fn left(n:&mut i32)->u8{*n=*n*10+1;3u8}fn right(n:&mut i32)->u8{*n=*n*10+2;4u8}fn run()->i32{let mut n=0;let x=same(left(&mut n),right(&mut n));if n==12 && x==4u8{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "argument_effects_once_ltr",
    "proposed_group": "positive",
    "source": "fn same<T>(a:T,b:T)->T{b}fn left(n:&mut i32)->u8{*n=*n*10+1;3u8}fn right(n:&mut i32)->u8{*n=*n*10+2;4u8}fn run()->i32{let mut n=0;let x=same(left(&mut n),right(&mut n));if n==12 && x==4u8{7}else{0}}",
    "source_bytes": 256,
    "source_sha256": "c4e217db20b60ce151b70efbece5eb2c78abfe146470bb64bbbb31ef21c3d77f",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}struct W<T>{value:T}fn run()->i32{let a=id::<u8>(3u8);let b=id(4u8);let x=W::<u16>{value:3u16};let y=W{value:4u16};if a+b==7u8 && x.value+y.value==7u16{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "fully_explicit_compatibility",
    "proposed_group": "positive",
    "source": "fn id<T>(x:T)->T{x}struct W<T>{value:T}fn run()->i32{let a=id::<u8>(3u8);let b=id(4u8);let x=W::<u16>{value:3u16};let y=W{value:4u16};if a+b==7u8 && x.value+y.value==7u16{7}else{0}}",
    "source_bytes": 239,
    "source_sha256": "4c2db0ec2a9bae948ee4b549248ed2626ad54a5d27fa7526f2727329366eeddd",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn same<T>(a:T,b:T)->T{b}fn run()->i32{let x=same(id(1),2u8);if x==2u8{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "opaque_nested_with_other_evidence",
    "proposed_group": "positive",
    "source": "fn id<T>(x:T)->T{x}fn same<T>(a:T,b:T)->T{b}fn run()->i32{let x=same(id(1),2u8);if x==2u8{7}else{0}}",
    "source_bytes": 158,
    "source_sha256": "da17c6e0084a1b2f57e1d4ab8c9ab4ecb098d9651ff8f419d21195723f7dd714",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn same<T>(a:T,b:T)->T{b}fn run()->i32{let mut n=0;let x:u8=id({n=n+1;7});let y:u8=same(1,{n=n+1;let x=2u8;x});let z:u8=id(if true{7}else{0});if n==2 && x==7u8 && y==2u8 && z==7u8{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "opaque_effect_argument_hard_context",
    "proposed_group": "positive",
    "source": "fn id<T>(x:T)->T{x}fn same<T>(a:T,b:T)->T{b}fn run()->i32{let mut n=0;let x:u8=id({n=n+1;7});let y:u8=same(1,{n=n+1;let x=2u8;x});let z:u8=id(if true{7}else{0});if n==2 && x==7u8 && y==2u8 && z==7u8{7}else{0}}",
    "source_bytes": 267,
    "source_sha256": "86057f4c4f32ca86beb5459378c6068bb6ca37a4f1c0217dd9d175afe8d88b27",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let a=id(1)+2u8;let b=1+id(2u8);if a==3u8 && b==3u8{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "positive",
    "name": "symmetric_binary_peer_hints",
    "proposed_group": "positive",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let a=id(1)+2u8;let b=1+id(2u8);if a==3u8 && b==3u8{7}else{0}}",
    "source_bytes": 153,
    "source_sha256": "1f0909213b1ae752a4bb329e879d0e2580a98657d389ec30c2fd67a71e2e87bb",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": "E0308",
    "expected_stdout": "",
    "full_source": "fn same<T>(a:T,b:T)->T{b}fn run()->i32{let x=same(1u8,2u16);7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "contradictory_known_argument_types",
    "proposed_group": "negative",
    "source": "fn same<T>(a:T,b:T)->T{b}fn run()->i32{let x=same(1u8,2u16);7}",
    "source_bytes": 81,
    "source_sha256": "760637d8586cdd46f3326723541fd288c2256eeabb5d90bfe21316f69336f032",
    "stock_check_error_codes": [
      "E0308"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0308"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0308",
    "expected_stdout": "",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let x:u8=id(1u16);7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "hard_result_vs_known_argument_conflict",
    "proposed_group": "negative",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let x:u8=id(1u16);7}",
    "source_bytes": 72,
    "source_sha256": "3b9cd78aa20b5cc03a838e4e897fcf03ce85b9ae3b2f3e39a0f3f5dcad628bfe",
    "stock_check_error_codes": [
      "E0308"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0308"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0382",
    "expected_stdout": "",
    "full_source": "fn id<T>(x:T)->T{x}struct S{x:i32}fn run()->i32{let s=S{x:7};let t=id(s);s.x}\nfn main(){run();}\n",
    "group": "negative",
    "name": "noncopy_value_reused_after_inferred_move",
    "proposed_group": "negative",
    "source": "fn id<T>(x:T)->T{x}struct S{x:i32}fn run()->i32{let s=S{x:7};let t=id(s);s.x}",
    "source_bytes": 96,
    "source_sha256": "4a64ac9fd931b75a0f80c6913254d0f7bf457e87d4e4d7ee06e36a22a566f79b",
    "stock_check_error_codes": [
      "E0382"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0382"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0382",
    "expected_stdout": "",
    "full_source": "fn bad<T>(x:T)->(T,T){(x,x)}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unused_universal_duplicate_body",
    "proposed_group": "negative",
    "source": "fn bad<T>(x:T)->(T,T){(x,x)}fn run()->i32{7}",
    "source_bytes": 63,
    "source_sha256": "2bb7a91f207c7b593d7241cf14bfd74402a1ad2c19afec35b7c7481de018cbe9",
    "stock_check_error_codes": [
      "E0382"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0382"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "overflowing_literals",
    "expected_stdout": "",
    "full_source": "fn same<T>(a:T,b:T)->T{b}fn run()->i32{let x=same(256,2u8);7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "literal_out_of_range_after_later_width_constraint",
    "proposed_group": "negative",
    "source": "fn same<T>(a:T,b:T)->T{b}fn run()->i32{let x=same(256,2u8);7}",
    "source_bytes": 80,
    "source_sha256": "3bc74f89d90d45c756e4642cf9e417cd23995c9448b9921b03d5b8fe40d1b330",
    "stock_check_error_codes": [
      "overflowing_literals"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "overflowing_literals"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0282",
    "expected_stdout": "",
    "full_source": "fn empty<T>(){}fn run()->i32{empty();7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "genuinely_unconstrained_unused_slot_ambiguity",
    "proposed_group": "negative",
    "source": "fn empty<T>(){}fn run()->i32{empty();7}",
    "source_bytes": 58,
    "source_sha256": "f618e8d2c1dc88269ef65d6ac23e4a3852cb6e30f0f68cffef2236757b79d971",
    "stock_check_error_codes": [
      "E0282"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0282"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0308",
    "expected_stdout": "",
    "full_source": "struct A<T>{value:T}struct B<T>{value:T}fn same<T>(a:A<T>,b:A<T>)->T{b.value}fn run()->i32{let a=A::<u8>{value:3};let b=B::<u8>{value:4};let x=same(a,b);7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "distinct_nominal_owner_conflict",
    "proposed_group": "negative",
    "source": "struct A<T>{value:T}struct B<T>{value:T}fn same<T>(a:A<T>,b:A<T>)->T{b.value}fn run()->i32{let a=A::<u8>{value:3};let b=B::<u8>{value:4};let x=same(a,b);7}",
    "source_bytes": 174,
    "source_sha256": "cea56fd5991059a5f0a6307e7093e1d7e03bcaf01345e836d27bd4f5327a1875",
    "stock_check_error_codes": [
      "E0308"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0308"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0308",
    "expected_stdout": "",
    "full_source": "fn same<T>(a:(T,T),b:(T,T))->T{b.0}fn run()->i32{let x=same((1u8,2u8),(3u8,4u8,5u8));7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "structural_tuple_or_array_shape_conflict",
    "proposed_group": "negative",
    "source": "fn same<T>(a:(T,T),b:(T,T))->T{b.0}fn run()->i32{let x=same((1u8,2u8),(3u8,4u8,5u8));7}",
    "source_bytes": 106,
    "source_sha256": "b548cb1d063b2ff1a9dffd9073b5143f854484943bc7b49332912cf07e74cf71",
    "stock_check_error_codes": [
      "E0308"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0308"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{id::<_>(7)}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "partial_explicit_type_hole",
    "proposed_group": "legalunsupported",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{id::<_>(7)}",
    "source_bytes": 102,
    "source_sha256": "1c72c864cfdd67a8a83a9ddb11ace8dbd1e55bee8661855ae3d588b736a146f8",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "enum E<T>{None,Some(T)}fn take(e:E<i32>)->i32{match e{E::None=>7,E::Some(v)=>v}}fn run()->i32{let e=E::None;take(e)}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "local_future_use_required",
    "proposed_group": "legalunsupported",
    "source": "enum E<T>{None,Some(T)}fn take(e:E<i32>)->i32{match e{E::None=>7,E::Some(v)=>v}}fn run()->i32{let e=E::None;take(e)}",
    "source_bytes": 174,
    "source_sha256": "40a143a66f87bff6bd9fc8a4be34b74561c520588b7b65706a7dd170fb8fdae2",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let x=id(id(7));if x==7{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "unresolved_nested_omitted_without_other_evidence",
    "proposed_group": "legalunsupported",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let x=id(id(7));if x==7{7}else{0}}",
    "source_bytes": 125,
    "source_sha256": "89d190a4da6bd97a0abfaa7c8a8b9768e14d0ae82ca2c30b622f5c530b3c2ba0",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "inference oracle ok\n",
    "full_source": "fn same<T>(a:T,b:T)->T{b}fn run()->i32{let v=same(1,{let x=2u8;x});if v==2u8{7}else{0}}\nfn main(){if run()==7{println!(\"inference oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "opaque_effect_argument_no_context",
    "proposed_group": "legalunsupported",
    "source": "fn same<T>(a:T,b:T)->T{b}fn run()->i32{let v=same(1,{let x=2u8;x});if v==2u8{7}else{0}}",
    "source_bytes": 145,
    "source_sha256": "64927b36dc414b3a0ff6ed0bfa31c0afbe9eb8ac83ad78ced0281893ec777fca",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
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
    metadata = directory / "stock.rmeta"
    argv = [rustc, "--edition=2024", "--crate-name", "reference", "-A", "warnings",
            "-D", "overflowing_literals", "-D", "arithmetic_overflow", "-C",
            "overflow-checks=yes", "--error-format=json", source]
    result = process([*argv, "-o", output], directory, compile_timeout)
    check = process([*argv, "--emit=metadata", "-o", metadata], directory, compile_timeout)
    fixture = FIXTURES[index]

    def error_codes(receipt):
        codes = []
        for line in receipt["stderr"].splitlines():
            try:
                diagnostic = json.loads(line)
            except json.JSONDecodeError:
                continue
            if diagnostic.get("level") == "error" and diagnostic.get("code"):
                codes.append(diagnostic["code"]["code"])
        return codes
    row = {"record": "stock_case", "index": index, "name": case.name,
           "kind": case.kind, "source": case.source,
           "source_sha256": hashlib.sha256(case.source.encode()).hexdigest(),
           "stock_compile": result, "stock_check": check,
           "stock_compile_artifact": output.is_file(),
           "stock_check_artifact": metadata.is_file(),
           "stock_error_codes": error_codes(result),
           "stock_check_error_codes": error_codes(check), "failures": []}
    if case.kind == "invalid":
        if (not rejected(result) or output.exists() or not rejected(check)
                or metadata.exists()):
            row["failures"].append("invalid fixture must reject both stock phases without artifacts")
        if (row["stock_error_codes"] != fixture["stock_error_codes"]
                or row["stock_check_error_codes"] != fixture["stock_check_error_codes"]):
            row["failures"].append("stock diagnostic codes differ from frozen observed codes")
    elif (not accepted(result) or not output.is_file()
          or not accepted(check) or not metadata.is_file()):
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
            prefix="jr-generic-inference-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "generic-inference-v1",
                "case_count": len(cases),
                "case_counts": {kind: sum(c.kind == kind for c in cases)
                                for kind in ("valid", "invalid", "boundary")},
                "stock_only": args.stock_only,
                "workers": [] if args.stock_only else [1, 8],
                "modes": [] if args.stock_only else ["build", "check"], "edition": 2024,
                "reference_modes": ["build", "check", "run"] if args.stock_only else ["build", "run"],
                "rustc_path": str(rustc),
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
            raise SystemExit(f"{totals['failure']} generic-inference discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
