#!/usr/bin/env python3
"""Bounded required-trait/static-dispatch differential corpus, not full Rust.

Exact corrected frozen32 dictionaries, full_source/stdout and diagnostic lists.
Local no-argument required traits and explicit bound obligations only.
Defaults, generic traits/methods, borrowed receivers and Copy remain excluded.
Shared bounded Case/run_case/process compares supplied-native workers1/8
BUILD/CHECK without building a compiler. B29 static default was authorized
before freeze after the original by-value default body stock E0277.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "bd6a43c218ef8f2b6596ee2281ddd28fa30d948448794fe5b3ea7904d0d5c6ef"

# Authoritative frozen stock oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}trait Take{fn make(x:i32)->Self;fn consume(self)->Self;fn value(self)->i32;}impl Take for S{fn make(x:i32)->Self{S{x}}fn consume(self)->Self{self}fn value(self)->i32{self.x}}fn run()->i32{<S as Take>::make(7).consume().value()}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "required_static_value_and_root_Self_return",
    "proposed_group": "positive",
    "source": "struct S{x:i32}trait Take{fn make(x:i32)->Self;fn consume(self)->Self;fn value(self)->i32;}impl Take for S{fn make(x:i32)->Self{S{x}}fn consume(self)->Self{self}fn value(self)->i32{self.x}}fn run()->i32{<S as Take>::make(7).consume().value()}",
    "source_bytes": 297,
    "source_sha256": "5d023ac9e6d0a9d4aa23946365717f386f529ae702f2df7f0ea4b76dbb49b2fa",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}trait Add{fn add(self,n:i32)->i32;}impl Add for S{fn add(mut self,n:i32)->i32{self.x=self.x+n;self.x}}fn run()->i32{S{x:3}.add(4)}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "impl_mut_self_trait_self_conformance",
    "proposed_group": "positive",
    "source": "struct S{x:i32}trait Add{fn add(self,n:i32)->i32;}impl Add for S{fn add(mut self,n:i32)->i32{self.x=self.x+n;self.x}}fn run()->i32{S{x:3}.add(4)}",
    "source_bytes": 200,
    "source_sha256": "6c66ff9e29ba1917c1e8c77530073392b211a28b95cf6466c1c57961fe055393",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}trait Ops{fn first(self,n:i32)->i32;fn second(x:i32)->S;}impl Ops for S{fn first(self,n:i32)->i32{self.x+n}fn second(x:i32)->S{S{x}}}fn run()->i32{<S as Ops>::second(3).first(4)}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "multiple_required_methods_exact_signatures",
    "proposed_group": "positive",
    "source": "struct S{x:i32}trait Ops{fn first(self,n:i32)->i32;fn second(x:i32)->S;}impl Ops for S{fn first(self,n:i32)->i32{self.x+n}fn second(x:i32)->S{S{x}}}fn run()->i32{<S as Ops>::second(3).first(4)}",
    "source_bytes": 248,
    "source_sha256": "516be510f5a95c9f2584f283e6d238008980a2323a16c9aa38b4859d939ee27a",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct A{x:i32}struct B{x:i32}trait Take{fn take(self)->i32;}impl Take for A{fn take(self)->i32{self.x}}impl Take for B{fn take(self)->i32{self.x}}fn run()->i32{A{x:3}.take()+B{x:4}.take()}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "distinct_nominal_implementors_same_trait",
    "proposed_group": "positive",
    "source": "struct A{x:i32}struct B{x:i32}trait Take{fn take(self)->i32;}impl Take for A{fn take(self)->i32{self.x}}impl Take for B{fn take(self)->i32{self.x}}fn run()->i32{A{x:3}.take()+B{x:4}.take()}",
    "source_bytes": 244,
    "source_sha256": "83b9a80d10b6c58252657dea19f97b144124a91715c9aa512b8c03f04e4cd19d",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}trait A{fn a(self)->i32;}trait B{fn b(self)->i32;}impl A for S{fn a(self)->i32{self.x}}impl B for S{fn b(self)->i32{self.x}}fn both<T:A+B>(x:T,y:T)->i32{x.a()+y.b()}fn run()->i32{both::<S>(S{x:3},S{x:4})}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "distinct_traits_inline_multi_bounds",
    "proposed_group": "positive",
    "source": "struct S{x:i32}trait A{fn a(self)->i32;}trait B{fn b(self)->i32;}impl A for S{fn a(self)->i32{self.x}}impl B for S{fn b(self)->i32{self.x}}fn both<T:A+B>(x:T,y:T)->i32{x.a()+y.b()}fn run()->i32{both::<S>(S{x:3},S{x:4})}",
    "source_bytes": 274,
    "source_sha256": "f41cecdccff3b41058cb58eeef16d89e2f3a8d5786864a95ee77a1031101692c",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct W<T>{x:T}trait Value{fn value(self)->i32;}impl<T> Value for W<T>{fn value(self)->i32{7}}fn run()->i32{W::<u8>{x:1u8}.value()}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "generic_nominal_trait_impl",
    "proposed_group": "positive",
    "source": "struct W<T>{x:T}trait Value{fn value(self)->i32;}impl<T> Value for W<T>{fn value(self)->i32{7}}fn run()->i32{W::<u8>{x:1u8}.value()}",
    "source_bytes": 187,
    "source_sha256": "0f639975b21d553084a9344deb4f553d1d39e98fadfd6cd2bffb176c64ce83ee",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct W<T>{x:T}trait Replace{fn replace(self,other:Self)->Self;}impl<T> Replace for W<T>{fn replace(self,other:Self)->Self{other}}fn run()->i32{let w=W::<i32>{x:3}.replace(W::<i32>{x:7});w.x}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "generic_impl_signature_substitution",
    "proposed_group": "positive",
    "source": "struct W<T>{x:T}trait Replace{fn replace(self,other:Self)->Self;}impl<T> Replace for W<T>{fn replace(self,other:Self)->Self{other}}fn run()->i32{let w=W::<i32>{x:3}.replace(W::<i32>{x:7});w.x}",
    "source_bytes": 247,
    "source_sha256": "826fa5673f595d4de450ce182f0338245f34f5fba7c6e795667188db0940d7d6",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}trait Value{fn make(x:i32)->Self;fn value(self)->i32;}impl Value for S{fn make(x:i32)->Self{S{x}}fn value(self)->i32{self.x}}fn call<T:Value>(x:i32)->i32{T::make(x).value()}fn run()->i32{call::<S>(7)}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "universal_bound_T_path_and_unique_scoped_dot",
    "proposed_group": "positive",
    "source": "struct S{x:i32}trait Value{fn make(x:i32)->Self;fn value(self)->i32;}impl Value for S{fn make(x:i32)->Self{S{x}}fn value(self)->i32{self.x}}fn call<T:Value>(x:i32)->i32{T::make(x).value()}fn run()->i32{call::<S>(7)}",
    "source_bytes": 270,
    "source_sha256": "2565eb2ce84a5971a91d1225419512e54185cc4f1db509a67606213411b1e845",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}fn bounded<T:Value>(x:T)->i32{x.value()}fn forward<U:Value>(x:U)->i32{bounded::<U>(x)}fn run()->i32{forward::<S>(S{x:7})}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "rigid_bound_forwarding_direct_generic_call",
    "proposed_group": "positive",
    "source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}fn bounded<T:Value>(x:T)->i32{x.value()}fn forward<U:Value>(x:U)->i32{bounded::<U>(x)}fn run()->i32{forward::<S>(S{x:7})}",
    "source_bytes": 269,
    "source_sha256": "009244ba5104e903484d4a4091f4fa7ded55edfeece4bfa7546d27fbbbbfddd8",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn run()->i32{W::<S>{x:S{x:7}}.value()}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "conditional_impl_prerequisite_proof",
    "proposed_group": "positive",
    "source": "struct S{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn run()->i32{W::<S>{x:S{x:7}}.value()}",
    "source_bytes": 268,
    "source_sha256": "7fd5d3cf4d80608eea4dcad17d9daa23602106889aac05f101290bf7de699cfc",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn consume<T:Value>(x:T)->i32{<T as Value>::value(x)}fn run()->i32{consume::<W<W<S>>>(W::<W<S>>{x:W::<S>{x:S{x:7}}})}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "nested_generic_nominal_conditional_dispatch",
    "proposed_group": "positive",
    "source": "struct S{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn consume<T:Value>(x:T)->i32{<T as Value>::value(x)}fn run()->i32{consume::<W<W<S>>>(W::<W<S>>{x:W::<S>{x:S{x:7}}})}",
    "source_bytes": 346,
    "source_sha256": "731f854fc31bd013e84552f6a6ec733ec4b1280b1776ec42f98906e2d8b78a21",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}struct Pair{a:S,b:S}trait Take{fn take(self)->i32;}impl Take for Pair{fn take(self)->i32{let a=self.a;let b=self.b;a.x+b.x}}fn run()->i32{Pair{a:S{x:3},b:S{x:4}}.take()}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "noncopy_receiver_consumed_once",
    "proposed_group": "positive",
    "source": "struct S{x:i32}struct Pair{a:S,b:S}trait Take{fn take(self)->i32;}impl Take for Pair{fn take(self)->i32{let a=self.a;let b=self.b;a.x+b.x}}fn run()->i32{Pair{a:S{x:3},b:S{x:4}}.take()}",
    "source_bytes": 239,
    "source_sha256": "5edf95729ad38094afe8ab662d6a8de3185ec51fbbf1289e7b53daf20abd01f7",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}trait Combine{fn combine(self,a:i32,b:i32)->i32;}impl Combine for S{fn combine(self,a:i32,b:i32)->i32{self.x+a+b}}fn receiver(n:&mut i32)->S{*n=*n*10+1;S{x:1}}fn arg(n:&mut i32,d:i32)->i32{*n=*n*10+d;d}fn run()->i32{let mut n=0;let x=receiver(&mut n).combine(arg(&mut n,2),arg(&mut n,3));if n==123 && x==6{7}else{0}}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "receiver_before_arguments_once_ltr",
    "proposed_group": "positive",
    "source": "struct S{x:i32}trait Combine{fn combine(self,a:i32,b:i32)->i32;}impl Combine for S{fn combine(self,a:i32,b:i32)->i32{self.x+a+b}}fn receiver(n:&mut i32)->S{*n=*n*10+1;S{x:1}}fn arg(n:&mut i32,d:i32)->i32{*n=*n*10+d;d}fn run()->i32{let mut n=0;let x=receiver(&mut n).combine(arg(&mut n,2),arg(&mut n,3));if n==123 && x==6{7}else{0}}",
    "source_bytes": 386,
    "source_sha256": "eaecefb1a71d4b4f512d0dbbfba75e48b58e41b52b792fdf33cd3646cd0c7159",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}trait A{fn value(self)->i32;}trait B{fn value(self)->i32;}impl A for S{fn value(self)->i32{self.x}}impl B for S{fn value(self)->i32{self.x+1}}fn run()->i32{<S as A>::value(S{x:3})+<S as B>::value(S{x:3})}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "explicit_UFCS_dispatch",
    "proposed_group": "positive",
    "source": "struct S{x:i32}trait A{fn value(self)->i32;}trait B{fn value(self)->i32;}impl A for S{fn value(self)->i32{self.x}}impl B for S{fn value(self)->i32{self.x+1}}fn run()->i32{<S as A>::value(S{x:3})+<S as B>::value(S{x:3})}",
    "source_bytes": 274,
    "source_sha256": "247fb9c0e16f5a439cf99476dbc199315845dd51c1e00a1a940573eb0b372a6b",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "mod m{pub struct S{pub x:i32}pub trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}}use m::Value;fn run()->i32{m::S{x:7}.value()}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "public_trait_and_impl_module_visibility",
    "proposed_group": "positive",
    "source": "mod m{pub struct S{pub x:i32}pub trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}}use m::Value;fn run()->i32{m::S{x:7}.value()}",
    "source_bytes": 212,
    "source_sha256": "1e8dfa74ce1c585f02e54d61b8797d082109a2496ec3cd9746637d8637147635",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn unused<T:Value>(x:T)->i32{<T as Value>::value(x)}fn run()->i32{7}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "positive",
    "name": "valid_unused_bound_function_and_impl",
    "proposed_group": "positive",
    "source": "struct S{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn unused<T:Value>(x:T)->i32{<T as Value>::value(x)}fn run()->i32{7}",
    "source_bytes": 297,
    "source_sha256": "515c510fe06f75f9c7a9bd1183ecfe5a98f32a911c5f51b420b82deba7bad509",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": "E0046",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unused_impl_missing_required_item",
    "proposed_group": "negative",
    "source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{}fn run()->i32{7}",
    "source_bytes": 101,
    "source_sha256": "bc1b1940283af213e8c6d0e0e15bae5b0fe67cd9e7ec3b4794652fb1ac0acda1",
    "stock_check_error_codes": [
      "E0046"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0046"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0407",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}fn extra(self)->i32{self.x}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unused_impl_extra_item",
    "proposed_group": "negative",
    "source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}fn extra(self)->i32{self.x}}fn run()->i32{7}",
    "source_bytes": 155,
    "source_sha256": "396f87740d05a2a37095750a2bf26966c9678995a08b633164f0fe68f3c9f3bf",
    "stock_check_error_codes": [
      "E0407"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0407"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0053",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}trait Value{fn value(self,x:i32)->i32;}impl Value for S{fn value(self,x:u8)->i32{self.x}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unused_impl_parameter_mismatch",
    "proposed_group": "negative",
    "source": "struct S{x:i32}trait Value{fn value(self,x:i32)->i32;}impl Value for S{fn value(self,x:u8)->i32{self.x}}fn run()->i32{7}",
    "source_bytes": 139,
    "source_sha256": "90e513126d4e81891888f202fbabdf095f02f149d97ee04d9ac7f92e58ed32d9",
    "stock_check_error_codes": [
      "E0053"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0053"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0053",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->u8{7u8}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unused_impl_return_mismatch",
    "proposed_group": "negative",
    "source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->u8{7u8}}fn run()->i32{7}",
    "source_bytes": 124,
    "source_sha256": "f0c8702bc4b21da901e6ce374bf7007c802096dcd3b1fcb8c84d9ad55637d293",
    "stock_check_error_codes": [
      "E0053"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0053"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0186",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value()->i32{7}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unused_impl_receiver_kind_mismatch",
    "proposed_group": "negative",
    "source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value()->i32{7}}fn run()->i32{7}",
    "source_bytes": 119,
    "source_sha256": "ab1c2e599481944791ee22ec61a1fa27aa24a3234fabe54e3e5501193b3e5787",
    "stock_check_error_codes": [
      "E0186"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0186"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0428",
    "expected_stdout": "",
    "full_source": "trait Value{fn value(self)->i32;fn value(self)->i32;}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "duplicate_required_trait_item",
    "proposed_group": "negative",
    "source": "trait Value{fn value(self)->i32;fn value(self)->i32;}fn run()->i32{7}",
    "source_bytes": 88,
    "source_sha256": "ce771268893336a20ec36ef9070a113b8e1533ef1666dc28edc41da90a10a0e0",
    "stock_check_error_codes": [
      "E0428"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0428"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0119",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{3}}impl Value for S{fn value(self)->i32{4}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "duplicate_same_trait_same_type_impl",
    "proposed_group": "negative",
    "source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{3}}impl Value for S{fn value(self)->i32{4}}fn run()->i32{7}",
    "source_bytes": 163,
    "source_sha256": "87e77e0f500f954e13cc329d39293374fa30b20f90dce66e582e5299affc2110",
    "stock_check_error_codes": [
      "E0119"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0119"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0119",
    "expected_stdout": "",
    "full_source": "struct W<T>{x:T}trait Value{fn value(self)->i32;}impl<T> Value for W<T>{fn value(self)->i32{3}}impl Value for W<i32>{fn value(self)->i32{4}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "overlapping_generic_concrete_impl",
    "proposed_group": "negative",
    "source": "struct W<T>{x:T}trait Value{fn value(self)->i32;}impl<T> Value for W<T>{fn value(self)->i32{3}}impl Value for W<i32>{fn value(self)->i32{4}}fn run()->i32{7}",
    "source_bytes": 175,
    "source_sha256": "b53407a55154adced325bb1533957bd594fcbda59a0b8c4f73c56e62650fdeb2",
    "stock_check_error_codes": [
      "E0119"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0119"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0207",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl<T> Value for S{fn value(self)->i32{self.x}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unused_impl_unconstrained_parameter",
    "proposed_group": "negative",
    "source": "struct S{x:i32}trait Value{fn value(self)->i32;}impl<T> Value for S{fn value(self)->i32{self.x}}fn run()->i32{7}",
    "source_bytes": 131,
    "source_sha256": "5cd9aa9fb70e3c16e6ea39268774f4af1def81340677b9e4741234526a8c45a7",
    "stock_check_error_codes": [
      "E0207"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0207"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0405",
    "expected_stdout": "",
    "full_source": "fn unused<T:Missing>(x:T)->T{x}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unused_header_unknown_trait",
    "proposed_group": "negative",
    "source": "fn unused<T:Missing>(x:T)->T{x}fn run()->i32{7}",
    "source_bytes": 66,
    "source_sha256": "36bd90efbfe775c7e3d0c6c3416f8a8ee783b82efaab30c3a9b3535a89bff1b6",
    "stock_check_error_codes": [
      "E0405"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0405"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0382",
    "expected_stdout": "",
    "full_source": "trait Value{fn value(self)->i32;}fn bad<T:Value>(x:T)->(T,T){(x,x)}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "universal_body_duplicate_move",
    "proposed_group": "negative",
    "source": "trait Value{fn value(self)->i32;}fn bad<T:Value>(x:T)->(T,T){(x,x)}fn run()->i32{7}",
    "source_bytes": 102,
    "source_sha256": "e2cea3af63b2319b491004afb522b2565c1ad65bdd52939f8c53ece5c9dc3862",
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
    "expected": "E0277",
    "expected_stdout": "",
    "full_source": "struct U{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn bounded<T:Value>(x:T)->i32{x.value()}fn missing<T>(x:T)->i32{bounded::<T>(x)}fn dead(){if false{bounded::<U>(U{x:7});}}fn run()->i32{W::<U>{x:U{x:7}}.value()}\nfn main(){run();}\n",
    "group": "negative",
    "name": "missing_bound_direct_call_and_conditional_prerequisite",
    "proposed_group": "negative",
    "source": "struct U{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn bounded<T:Value>(x:T)->i32{x.value()}fn missing<T>(x:T)->i32{bounded::<T>(x)}fn dead(){if false{bounded::<U>(U{x:7});}}fn run()->i32{W::<U>{x:U{x:7}}.value()}",
    "source_bytes": 309,
    "source_sha256": "00b6261ba9eec0f330a6fe46d0e6920941ac64e35d43d56bd262c93529d86cc8",
    "stock_check_error_codes": [
      "E0277",
      "E0277",
      "E0599"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0277",
      "E0277",
      "E0599"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}trait Value{fn value()->i32{7}}impl Value for S{}fn run()->i32{<S as Value>::value()}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "default_method_body",
    "proposed_group": "legalunsupported",
    "source": "struct S{x:i32}trait Value{fn value()->i32{7}}impl Value for S{}fn run()->i32{<S as Value>::value()}",
    "source_bytes": 155,
    "source_sha256": "280005597742cea9f0de275d4177c682a3acd2c378d7d8dc8df99325e82de4e9",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}trait Value<T>{fn value(self,x:T)->i32;}impl Value<i32> for S{fn value(self,x:i32)->i32{self.x+x}}fn run()->i32{S{x:3}.value(4)}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "generic_trait_parameters",
    "proposed_group": "legalunsupported",
    "source": "struct S{x:i32}trait Value<T>{fn value(self,x:T)->i32;}impl Value<i32> for S{fn value(self,x:i32)->i32{self.x+x}}fn run()->i32{S{x:3}.value(4)}",
    "source_bytes": 198,
    "source_sha256": "47bc5a2ad33a79fc2e92796d8c8ec197d463021e7d3f8581c474422ab64203b0",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}trait Choose{fn choose<T>(self,x:T)->T;}impl Choose for S{fn choose<T>(self,x:T)->T{x}}fn run()->i32{S{x:0}.choose::<i32>(7)}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "generic_trait_method",
    "proposed_group": "legalunsupported",
    "source": "struct S{x:i32}trait Choose{fn choose<T>(self,x:T)->T;}impl Choose for S{fn choose<T>(self,x:T)->T{x}}fn run()->i32{S{x:0}.choose::<i32>(7)}",
    "source_bytes": 195,
    "source_sha256": "8ebfd4ad372ed80c2a24d56ce0f47cf4bc9a9ab08a45a4993685a0ba764c2536",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "traits oracle ok\n",
    "full_source": "struct S{x:i32}trait Read{fn read(&self)->i32;}impl Read for S{fn read(&self)->i32{self.x}}fn dup<T:Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let s=S{x:3};let p=dup::<i32>(2);s.read()+p.0+p.1}\nfn main(){if run()==7{println!(\"traits oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "borrowed_receiver_and_Copy_bound",
    "proposed_group": "legalunsupported",
    "source": "struct S{x:i32}trait Read{fn read(&self)->i32;}impl Read for S{fn read(&self)->i32{self.x}}fn dup<T:Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let s=S{x:3};let p=dup::<i32>(2);s.read()+p.0+p.1}",
    "source_bytes": 243,
    "source_sha256": "8e5659d9a873dff362565e49cd1755f0544f51b8c76c19f06ffe8b0ef58b2cc5",
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
    assert len(cases) == 32 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-traits-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "traits-v1",
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
            raise SystemExit(f"{totals['failure']} traits discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
