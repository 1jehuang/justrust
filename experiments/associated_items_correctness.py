#!/usr/bin/env python3
"""Bounded inherent associated-items differential corpus, not full Rust.

Exact frozen32 stock-classified source/stdout bytes. Current classification is
19 positive, 10 invalid and 3 legal unsupported after the authorized unchanged
trait_dispatch promotion. The acceptance hash retains ORIGINAL frozen provenance.
By-value dispatch with explicit method arguments and admitted required-trait
static dispatch are supported. Borrowed receiver adjustments, omitted method
arguments and partial holes remain legal unsupported boundaries.
Shared bounded Case/run_case/process
compares supplied-native workers1/8 BUILD/CHECK, never builds a compiler.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "53a4ca56370f69e590af9328659254acf6500220c47388029145ca3f047f48fa"

# Authoritative frozen stock oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn make(x:i32)->S{S{x}}}fn run()->i32{S::make(7).x}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "associated_function_struct",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn make(x:i32)->S{S{x}}}fn run()->i32{S::make(7).x}",
    "source_bytes": 138,
    "source_sha256": "fb79ce74748d6350093a7c40a4b22e6c652fff1fbcaf26f63fedc7911fcb4223",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "enum E{A(i32),B}impl E{fn make(x:i32)->E{E::A(x)}}fn run()->i32{match E::make(7){E::A(x)=>x,E::B=>0}}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "associated_function_enum",
    "proposed_group": "positive",
    "source": "enum E{A(i32),B}impl E{fn make(x:i32)->E{E::A(x)}}fn run()->i32{match E::make(7){E::A(x)=>x,E::B=>0}}",
    "source_bytes": 166,
    "source_sha256": "9c9e92390fbab48733e870cc2713230977f6958e21ef2b3965a220713ae949f5",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn take(self)->i32{self.x}}fn run()->i32{S{x:3}.take()+S::take(S{x:4})}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "consuming_self_dot_and_qualified_equivalence",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn take(self)->i32{self.x}}fn run()->i32{S{x:3}.take()+S::take(S{x:4})}",
    "source_bytes": 158,
    "source_sha256": "b484b675846094c53cc0b379d3985793942ba5ebf8df87f01b0abf666270b35b",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn add(mut self,n:i32)->i32{self.x=self.x+n;self.x}}fn run()->i32{let s=S{x:3};s.add(4)}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "mut_self_local_mutation",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn add(mut self,n:i32)->i32{self.x=self.x+n;self.x}}fn run()->i32{let s=S{x:3};s.add(4)}",
    "source_bytes": 175,
    "source_sha256": "c851cb1e98b7dc305b292616a34e118e03b11df4a278dfdfe0cddd7bbb2406f2",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct W<T>{value:T}impl<T> W<T>{fn take(self)->T{self.value}fn make(value:T)->W<T>{W::<T>{value}}}fn run()->i32{W::<i32>::make(7).take()}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "impl_binder_receiver_matching",
    "proposed_group": "positive",
    "source": "struct W<T>{value:T}impl<T> W<T>{fn take(self)->T{self.value}fn make(value:T)->W<T>{W::<T>{value}}}fn run()->i32{W::<i32>::make(7).take()}",
    "source_bytes": 203,
    "source_sha256": "2debcc57d3976cbb6f26c846623248395c68033b0c96a58271a63296b93048dc",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn choose<U>(self,u:U)->U{u}fn identity<U>(u:U)->U{u}}fn run()->i32{let a=S{x:0}.choose::<i32>(3);let b=S::identity::<i32>(4);a+b}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "method_binder_explicit_arguments",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn choose<U>(self,u:U)->U{u}fn identity<U>(u:U)->U{u}}fn run()->i32{let a=S{x:0}.choose::<i32>(3);let b=S::identity::<i32>(4);a+b}",
    "source_bytes": 217,
    "source_sha256": "8f74bf66eec2524f5e66eadda6d7f6db436fc79cfeaf96c47faaea8f6febfa61",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct Pair<T,U>{a:T,b:U}struct W<T>{value:T}impl<T> W<T>{fn pair<U>(self,u:U)->Pair<U,T>{Pair::<U,T>{a:u,b:self.value}}}fn forward<T,U>(a:T,b:U)->Pair<U,T>{W::<T>{value:a}.pair::<U>(b)}fn run()->i32{let p=forward::<i32,u8>(3,4u8);p.a as i32+p.b}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "impl_method_distinct_binders_forwarding",
    "proposed_group": "positive",
    "source": "struct Pair<T,U>{a:T,b:U}struct W<T>{value:T}impl<T> W<T>{fn pair<U>(self,u:U)->Pair<U,T>{Pair::<U,T>{a:u,b:self.value}}}fn forward<T,U>(a:T,b:U)->Pair<U,T>{W::<T>{value:a}.pair::<U>(b)}fn run()->i32{let p=forward::<i32,u8>(3,4u8);p.a as i32+p.b}",
    "source_bytes": 311,
    "source_sha256": "7bc199fd4a3eefb0b909721a56535fc8077dad6f8391382968ec0796713cda9d",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct A<T>{x:T}struct B<T>{x:T}impl<T> A<T>{fn take(self)->T{self.x}}impl<T> B<T>{fn take(self)->T{self.x}}fn run()->i32{let a=A::<u8>{x:3u8}.take();let b=B::<i32>{x:4}.take();a as i32+b}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "unrelated_same_spelled_binder_identity",
    "proposed_group": "positive",
    "source": "struct A<T>{x:T}struct B<T>{x:T}impl<T> A<T>{fn take(self)->T{self.x}}impl<T> B<T>{fn take(self)->T{self.x}}fn run()->i32{let a=A::<u8>{x:3u8}.take();let b=B::<i32>{x:4}.take();a as i32+b}",
    "source_bytes": 253,
    "source_sha256": "fb63493f2721489ff028986b83b10158788510264dbabe7f580aceb82d498736",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn make(x:i32)->Self{Self{x}}fn via(x:i32)->Self{Self::make(x)}fn take(self)->i32{self.x}}enum E{Item(i32)}impl E{fn make(x:i32)->Self{Self::Item(x)}fn take(self)->i32{match self{Self::Item(x)=>x}}}fn run()->i32{S::via(3).take()+E::make(4).take()}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "Self_type_constructor_and_associated_path",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn make(x:i32)->Self{Self{x}}fn via(x:i32)->Self{Self::make(x)}fn take(self)->i32{self.x}}enum E{Item(i32)}impl E{fn make(x:i32)->Self{Self::Item(x)}fn take(self)->i32{match self{Self::Item(x)=>x}}}fn run()->i32{S::via(3).take()+E::make(4).take()}",
    "source_bytes": 334,
    "source_sha256": "86fa6f3b466d2dc1a63200ed75400890fe369a8d4165d7d3e33be1affccc8308",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct W<T>{x:T}impl W<u8>{fn score(self)->i32{1}}impl W<u16>{fn score(self)->i32{2}}struct Pair<T,U>{a:T,b:U}impl<T> Pair<T,T>{fn score(self)->i32{3}}impl Pair<i32,u8>{fn score(self)->i32{1}}fn run()->i32{W::<u8>{x:0u8}.score()+W::<u16>{x:0u16}.score()+Pair::<i32,i32>{a:0,b:0}.score()+Pair::<i32,u8>{a:0,b:0u8}.score()}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "disjoint_concrete_impl_same_method",
    "proposed_group": "positive",
    "source": "struct W<T>{x:T}impl W<u8>{fn score(self)->i32{1}}impl W<u16>{fn score(self)->i32{2}}struct Pair<T,U>{a:T,b:U}impl<T> Pair<T,T>{fn score(self)->i32{3}}impl Pair<i32,u8>{fn score(self)->i32{1}}fn run()->i32{W::<u8>{x:0u8}.score()+W::<u16>{x:0u16}.score()+Pair::<i32,i32>{a:0,b:0}.score()+Pair::<i32,u8>{a:0,b:0u8}.score()}",
    "source_bytes": 386,
    "source_sha256": "3d5d198e3202977a5e228369cd7848f66fa90a41dcdcc89f81c05e444149b76e",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct W<T>{x:T}impl<T> W<T>{fn take(self)->T{self.x}}impl W<i32>{fn make(x:i32)->W<i32>{W::<i32>{x}}}fn run()->i32{W::<i32>::make(7).take()}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "separate_impl_disjoint_item_names",
    "proposed_group": "positive",
    "source": "struct W<T>{x:T}impl<T> W<T>{fn take(self)->T{self.x}}impl W<i32>{fn make(x:i32)->W<i32>{W::<i32>{x}}}fn run()->i32{W::<i32>::make(7).take()}",
    "source_bytes": 206,
    "source_sha256": "c6d87f4f97993c64d9375c08c6b14a7827e048ed4c8ac6a185099da16ba9b3be",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "mod a{pub struct S{pub x:i32}impl S{pub fn make(x:i32)->S{S{x}}pub fn take(self)->i32{self.x}}}mod b{pub struct S{pub x:i32}impl S{pub fn make(x:i32)->S{S{x}}pub fn take(self)->i32{self.x}}}fn run()->i32{a::S::make(3).take()+b::S::make(4).take()}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "nominal_owner_identity_same_item_name",
    "proposed_group": "positive",
    "source": "mod a{pub struct S{pub x:i32}impl S{pub fn make(x:i32)->S{S{x}}pub fn take(self)->i32{self.x}}}mod b{pub struct S{pub x:i32}impl S{pub fn make(x:i32)->S{S{x}}pub fn take(self)->i32{self.x}}}fn run()->i32{a::S::make(3).take()+b::S::make(4).take()}",
    "source_bytes": 311,
    "source_sha256": "c181cf1ee91be99c16a1816c6c69a127ea87054815edc635f3066df73e777569",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn combine(self,a:i32,b:i32)->i32{self.x+a+b}}fn receiver(n:&mut i32)->S{*n=*n*10+1;S{x:1}}fn arg(n:&mut i32,d:i32)->i32{*n=*n*10+d;d}fn run()->i32{let mut n=0;let x=receiver(&mut n).combine(arg(&mut n,2),arg(&mut n,3));if n==123 && x==6{7}else{0}}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "receiver_then_arguments_once_ltr",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn combine(self,a:i32,b:i32)->i32{self.x+a+b}}fn receiver(n:&mut i32)->S{*n=*n*10+1;S{x:1}}fn arg(n:&mut i32,d:i32)->i32{*n=*n*10+d;d}fn run()->i32{let mut n=0;let x=receiver(&mut n).combine(arg(&mut n,2),arg(&mut n,3));if n==123 && x==6{7}else{0}}",
    "source_bytes": 335,
    "source_sha256": "3474db370b2901f7a6e971b0b5f26b13ef6c747c5dce93fa7d6c605f6c8c4489",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct S{x:i32}struct P{a:S,b:S}impl P{fn take(self)->i32{let a=self.a;let b=self.b;a.x+b.x}}fn run()->i32{P{a:S{x:3},b:S{x:4}}.take()}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "noncopy_receiver_partial_field_moves",
    "proposed_group": "positive",
    "source": "struct S{x:i32}struct P{a:S,b:S}impl P{fn take(self)->i32{let a=self.a;let b=self.b;a.x+b.x}}fn run()->i32{P{a:S{x:3},b:S{x:4}}.take()}",
    "source_bytes": 200,
    "source_sha256": "7a8541edaa3626c662a8403e010ebd4691f82da74ac310b8d57c308322e5d7c8",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct W<T>{x:T}impl<T> W<T>{fn pair<U>(self,u:U)->(T,U){(self.x,u)}fn forward<U>(self,u:U)->(T,U){self.pair::<U>(u)}}fn bridge<T,U>(x:T,u:U)->(T,U){W::<T>{x}.forward::<U>(u)}fn run()->i32{let p=bridge::<u8,i32>(3u8,4);p.0 as i32+p.1}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "rigid_generic_method_forwarding",
    "proposed_group": "positive",
    "source": "struct W<T>{x:T}impl<T> W<T>{fn pair<U>(self,u:U)->(T,U){(self.x,u)}fn forward<U>(self,u:U)->(T,U){self.pair::<U>(u)}}fn bridge<T,U>(x:T,u:U)->(T,U){W::<T>{x}.forward::<U>(u)}fn run()->i32{let p=bridge::<u8,i32>(3u8,4);p.0 as i32+p.1}",
    "source_bytes": 299,
    "source_sha256": "16a13210d773b00ab5d04ef55c7e7ba98d82af351d18f7786bb46de509c26377",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct W<T>{x:T}impl<T> W<T>{fn recur<U>(self,u:U,n:i32)->(T,U){if n==0{(self.x,u)}else{self.recur::<U>(u,n-1)}}}fn run()->i32{let p=W::<i32>{x:3}.recur::<u8>(4u8,3);p.0+p.1 as i32}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "exact_instance_method_recursion",
    "proposed_group": "positive",
    "source": "struct W<T>{x:T}impl<T> W<T>{fn recur<U>(self,u:U,n:i32)->(T,U){if n==0{(self.x,u)}else{self.recur::<U>(u,n-1)}}}fn run()->i32{let p=W::<i32>{x:3}.recur::<u8>(4u8,3);p.0+p.1 as i32}",
    "source_bytes": 246,
    "source_sha256": "1fc6f9fcb03a9c550e71c0b1f0e87bf89c751d2d1a19bb61e0150ed4749f5945",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn take<U>(self)->i32{self.x}fn never_used<V>(x:i32)->i32{x}}fn run()->i32{S{x:7}.take::<u64>()}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "unused_method_type_parameter_explicit",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn take<U>(self)->i32{self.x}fn never_used<V>(x:i32)->i32{x}}fn run()->i32{S{x:7}.take::<u64>()}",
    "source_bytes": 183,
    "source_sha256": "c0e40719607e6235127a04931742b4173d72f21eb691a795a9e006610c7f9ff3",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "mod a{pub struct S{x:i32}impl S{pub fn make(x:i32)->S{S{x}}pub fn take(self)->i32{self.x}}}mod b{impl super::a::S{pub fn consume(self)->i32{self.take()}}}fn run()->i32{a::S::make(7).consume()}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "visible_public_method_across_module",
    "proposed_group": "positive",
    "source": "mod a{pub struct S{x:i32}impl S{pub fn make(x:i32)->S{S{x}}pub fn take(self)->i32{self.x}}}mod b{impl super::a::S{pub fn consume(self)->i32{self.take()}}}fn run()->i32{a::S::make(7).consume()}",
    "source_bytes": 257,
    "source_sha256": "2172a0fad429acd52b6ff0d71be1b9a4175571c0a18e035d2f53df4833a876b6",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": "E0382",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}impl S{fn take(self)->i32{self.x}}fn run()->i32{let s=S{x:7};let n=s.take();s.x}\nfn main(){run();}\n",
    "group": "negative",
    "name": "use_after_consuming_receiver",
    "proposed_group": "negative",
    "source": "struct S{x:i32}impl S{fn take(self)->i32{self.x}}fn run()->i32{let s=S{x:7};let n=s.take();s.x}",
    "source_bytes": 114,
    "source_sha256": "da70a4174e5b4c44fa0dfcfc49df70eca83d2addf07ef35277ab7db82a6d3348",
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
    "full_source": "struct W<T>{x:T}impl<T> W<T>{fn bad<U>(self,u:U)->(U,U){(u,u)}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unused_generic_method_duplicate_move",
    "proposed_group": "negative",
    "source": "struct W<T>{x:T}impl<T> W<T>{fn bad<U>(self,u:U)->(U,U){(u,u)}}fn run()->i32{7}",
    "source_bytes": 98,
    "source_sha256": "6f0dd7a68e31e753ae8763438a3f28a2e07160ceb7cb1107e3eb63b8d5c91314",
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
    "expected": "E0308",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}impl S{fn bad<U>(self)->U{7}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unused_method_type_error",
    "proposed_group": "negative",
    "source": "struct S{x:i32}impl S{fn bad<U>(self)->U{7}}fn run()->i32{7}",
    "source_bytes": 79,
    "source_sha256": "60ddc1737b8e2e19c5f0cd75d56e7cbf471d34d5f27cdb53d2cf2959f97572d5",
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
    "expected": "E0207",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}impl<T> S{fn unused(self)->i32{self.x}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unconstrained_impl_parameter_header",
    "proposed_group": "negative",
    "source": "struct S{x:i32}impl<T> S{fn unused(self)->i32{self.x}}fn run()->i32{7}",
    "source_bytes": 89,
    "source_sha256": "277d5f9e528bf7eafc30665dbd9088deda39dd7d600542700c279f3a454d1faa",
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
    "expected": "E0403",
    "expected_stdout": "",
    "full_source": "struct W<T>{x:T}impl<T> W<T>{fn bad<T>(self,x:T)->T{x}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "duplicate_visible_impl_method_parameter_name",
    "proposed_group": "negative",
    "source": "struct W<T>{x:T}impl<T> W<T>{fn bad<T>(self,x:T)->T{x}}fn run()->i32{7}",
    "source_bytes": 90,
    "source_sha256": "6c38d2972349e1623431fe39c74e267a61d0a31c87feee7b39dcdd14f93c6a27",
    "stock_check_error_codes": [
      "E0403"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0403"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0592",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}impl S{fn clash(self)->i32{3}fn clash(self)->i32{4}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "duplicate_items_within_impl",
    "proposed_group": "negative",
    "source": "struct S{x:i32}impl S{fn clash(self)->i32{3}fn clash(self)->i32{4}}fn run()->i32{7}",
    "source_bytes": 102,
    "source_sha256": "716225dbbde3d338fce0d351ea77b902c7e5d08ca608112edd8db7ea6a266b14",
    "stock_check_error_codes": [
      "E0592"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0592"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0592",
    "expected_stdout": "",
    "full_source": "struct W<T>{x:T}impl<T> W<T>{fn clash(self)->i32{3}}impl W<i32>{fn clash(self)->i32{4}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "overlapping_generic_concrete_same_named_item",
    "proposed_group": "negative",
    "source": "struct W<T>{x:T}impl<T> W<T>{fn clash(self)->i32{3}}impl W<i32>{fn clash(self)->i32{4}}fn run()->i32{7}",
    "source_bytes": 122,
    "source_sha256": "bbd83e4c203369440fe941b44ee8a6e0093d92705f8883cbf12472ae072cb743",
    "stock_check_error_codes": [
      "E0592"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0592"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0624",
    "expected_stdout": "",
    "full_source": "mod a{pub struct S{pub x:i32}impl S{fn take(self)->i32{self.x}}}fn run()->i32{a::S{x:7}.take()}\nfn main(){run();}\n",
    "group": "negative",
    "name": "private_method_access",
    "proposed_group": "negative",
    "source": "mod a{pub struct S{pub x:i32}impl S{fn take(self)->i32{self.x}}}fn run()->i32{a::S{x:7}.take()}",
    "source_bytes": 114,
    "source_sha256": "cf4626a11d9b69878d61927bd5f011b8a29278dca279388fb71693712c2b4a14",
    "stock_check_error_codes": [
      "E0624"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0624"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0616",
    "expected_stdout": "",
    "full_source": "mod a{pub struct S{x:i32}}mod b{impl super::a::S{pub fn leak(self)->i32{self.x}}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "impl_site_cannot_access_definition_private_field",
    "proposed_group": "negative",
    "source": "mod a{pub struct S{x:i32}}mod b{impl super::a::S{pub fn leak(self)->i32{self.x}}}fn run()->i32{7}",
    "source_bytes": 116,
    "source_sha256": "2c9432299cc21d748e5b99f6de99bdbe0c3a5b6798f823c0acf54e2487d200b8",
    "stock_check_error_codes": [
      "E0616"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0616"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0599",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}impl S{fn make(x:i32)->S{S{x}}fn take(self)->i32{self.x}}fn run()->i32{let s=S{x:7};s.make(1);S::take();S::make(1,2);7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "associated_and_receiver_call_arity_misuse",
    "proposed_group": "negative",
    "source": "struct S{x:i32}impl S{fn make(x:i32)->S{S{x}}fn take(self)->i32{self.x}}fn run()->i32{let s=S{x:7};s.make(1);S::take();S::make(1,2);7}",
    "source_bytes": 153,
    "source_sha256": "74c0d5978c0944b89d4bded68709d8f8fd75455b2e787b0b8d3a4c81946fd312",
    "stock_check_error_codes": [
      "E0599",
      "E0061",
      "E0061"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0599",
      "E0061",
      "E0061"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct S{x:i32}trait Take{fn take(self)->i32;}impl Take for S{fn take(self)->i32{self.x}}fn run()->i32{S{x:7}.take()}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "positive",
    "name": "trait_dispatch",
    "proposed_group": "legalunsupported",
    "source": "struct S{x:i32}trait Take{fn take(self)->i32;}impl Take for S{fn take(self)->i32{self.x}}fn run()->i32{S{x:7}.take()}",
    "source_bytes": 182,
    "source_sha256": "2b892fda952e7215f317df893b69d7fc2f079c3bcf8b2c393e8d4fce5d66d71b",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn run()->i32{let s=S{x:3};let a=s.get();let r=&s;a+r.get()+1}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "borrowed_receiver_autoref_autoderef",
    "proposed_group": "legalunsupported",
    "source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn run()->i32{let s=S{x:3};let a=s.get();let r=&s;a+r.get()+1}",
    "source_bytes": 176,
    "source_sha256": "825ef0e1000e4befc7ec9cfa5a778e0eda695348d8a88e0c0b9a22fe3387646a",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn choose<U>(self,u:U)->U{u}}fn run()->i32{S{x:0}.choose(7)}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "omitted_method_type_arguments",
    "proposed_group": "legalunsupported",
    "source": "struct S{x:i32}impl S{fn choose<U>(self,u:U)->U{u}}fn run()->i32{S{x:0}.choose(7)}",
    "source_bytes": 147,
    "source_sha256": "72d76fed46287ad36082f27af3a3593caef373dc09b1c4a05306e5626de3535b",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "associated items oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn choose<U>(self,u:U)->U{u}}fn run()->i32{S{x:0}.choose::<_>(7)}\nfn main(){if run()==7{println!(\"associated items oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "partial_method_type_hole",
    "proposed_group": "legalunsupported",
    "source": "struct S{x:i32}impl S{fn choose<U>(self,u:U)->U{u}}fn run()->i32{S{x:0}.choose::<_>(7)}",
    "source_bytes": 152,
    "source_sha256": "ee7c40d56de9eeccf1f5f5fcad843150684e115a14c38da909da97eb20a937dd",
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
            prefix="jr-associated-items-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "associated-items-v1",
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
            raise SystemExit(f"{totals['failure']} associated-items discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
