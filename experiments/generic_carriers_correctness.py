"""Frozen exact32 executable generic carrier differential corpus.

Sixteen positives, twelve invalids and four legal exclusions. Preserves exact
source dictionaries, uses a supplied native binary with workers1/8, and never
builds it. Stock CHECK is metadata-only with actual regular rmeta presence.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case

ACCEPTANCE_CORPUS_SHA256 = "400bf44235e1c5a34ff964a10ab61111696d7765010a3e66bd1d1bcf602f8842"

# Exact observed dictionary bytes, with separate declaration/full-source hashes.
FROZEN_CORPUS_JSON = r'''[
  {
    "id": "P01",
    "name": "explicit_shared_root_actual_identity",
    "group": "positive",
    "proposed_group": "positive",
    "category": "root-family transport",
    "status": "valid",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let v=1;let r=id::<&i32>(&v);*r}",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let v=1;let r=id::<&i32>(&v);*r}\nfn main(){if run()==1{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 1,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "11ca46a9a72fb38fe0baf6680e9bdac677a41bbc6e0e796dfac4158ae45b05d1",
    "full_source_sha256": "ebaed1b3ccbef0d3cd5746e2d39bd3bf3d3eb4088d14b1a867d1c0d4a71bc164",
    "source_bytes": 65,
    "full_source_bytes": 130,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P02",
    "name": "rigid_parameter_forwarding_preserves_hidden_origins",
    "group": "positive",
    "proposed_group": "positive",
    "category": "rigid forwarding",
    "status": "valid",
    "source": "fn id<T>(x:T)->T{x}fn forward<T>(x:T)->T{id::<T>(x)}fn run()->i32{let v=2;let r=forward::<&i32>(&v);*r}",
    "full_source": "fn id<T>(x:T)->T{x}fn forward<T>(x:T)->T{id::<T>(x)}fn run()->i32{let v=2;let r=forward::<&i32>(&v);*r}\nfn main(){if run()==2{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 2,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "dba108df3aee00a92c64d335d85c6c8d4cd3253c7d540d358499b132302c96dc",
    "full_source_sha256": "49bafa3f7d6a07f9dddb7e9dd97d0b7c223ee2cd571537c723d6383412461b9c",
    "source_bytes": 103,
    "full_source_bytes": 168,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P03",
    "name": "shared_root_copy_without_referent_copy",
    "group": "positive",
    "proposed_group": "positive",
    "category": "exact-context Copy",
    "status": "valid",
    "source": "struct S{v:i32}fn twice<T:Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let s=S{v:3};let pair=twice::<&S>(&s);let a=pair.0.v;let b=pair.1.v;a+b}",
    "full_source": "struct S{v:i32}fn twice<T:Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let s=S{v:3};let pair=twice::<&S>(&s);let a=pair.0.v;let b=pair.1.v;a+b}\nfn main(){if run()==6{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 6,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "aed81ad7d352b36963db15884f0597d3588de6ad2c08ef7c610affd60d49a17b",
    "full_source_sha256": "c68b9aebf9f83394743b8806b79a75292bb2237d2ced4f0d620ad739d2bafc80",
    "source_bytes": 136,
    "full_source_bytes": 201,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P04",
    "name": "single_affine_mutable_root_identity_transfer",
    "group": "positive",
    "proposed_group": "positive",
    "category": "one affine result",
    "status": "valid",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let mut v=0;let r=id::<&mut i32>(&mut v);*r=4;v}",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let mut v=0;let r=id::<&mut i32>(&mut v);*r=4;v}\nfn main(){if run()==4{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 4,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "aae1dcac11b67cbfb065dd9ed01558d89bf56b3650fc3ffc5f7c4f883a47cfee",
    "full_source_sha256": "89971b9a6a0ece69abe1e29464c426e7cb1c8edc1fc3325dccdd3fc8cdf6c072",
    "source_bytes": 81,
    "full_source_bytes": 146,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P05",
    "name": "internal_tuple_components_keep_distinct_occurrence_origins",
    "group": "positive",
    "proposed_group": "positive",
    "category": "typed tuple paths",
    "status": "valid",
    "source": "fn pair<T>(x:T,y:T)->(T,T){(x,y)}fn run()->i32{let a=5;let b=6;let p=pair::<&i32>(&a,&b);let first=*p.0;let second=*p.1;first*100+second}",
    "full_source": "fn pair<T>(x:T,y:T)->(T,T){(x,y)}fn run()->i32{let a=5;let b=6;let p=pair::<&i32>(&a,&b);let first=*p.0;let second=*p.1;first*100+second}\nfn main(){if run()==506{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 506,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "d865f03f5c05c47e1857491512644e5863a070c1e6b489cfc62c7c3d58493bd6",
    "full_source_sha256": "1b9cd35314caac2c0cac682ade8a63a42111aece22c2f65b2e77c3637cb947a9",
    "source_bytes": 137,
    "full_source_bytes": 204,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P06",
    "name": "internal_array_constant_index_component_transport",
    "group": "positive",
    "proposed_group": "positive",
    "category": "typed array paths",
    "status": "valid",
    "source": "fn pair<T>(x:T,y:T)->[T;2]{[x,y]}fn run()->i32{let a=6;let b=7;let p=pair::<&i32>(&a,&b);let first=*p[0];let second=*p[1];first*100+second}",
    "full_source": "fn pair<T>(x:T,y:T)->[T;2]{[x,y]}fn run()->i32{let a=6;let b=7;let p=pair::<&i32>(&a,&b);let first=*p[0];let second=*p[1];first*100+second}\nfn main(){if run()==607{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 607,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "2c428590d8ca610bcd66fb2c72405b2039cc604ea7a111fbd20ad69f19f73f89",
    "full_source_sha256": "5ae53f87f3c30981dfadd49311f73cc279d87ba819f175a883e259e1c7893762",
    "source_bytes": 139,
    "full_source_bytes": 206,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P07",
    "name": "internal_nominal_variant_component_correspondence",
    "group": "positive",
    "proposed_group": "positive",
    "category": "typed nominal and variant mapping",
    "status": "valid",
    "source": "struct Wrap<T>{value:T}enum E<T>{One{value:T}}fn wrap<T>(x:T)->E<T>{let w=Wrap{value:x};E::One{value:w.value}}fn run()->i32{let v=7;let e=wrap::<&i32>(&v);match e{E::One{value}=>*value}}",
    "full_source": "struct Wrap<T>{value:T}enum E<T>{One{value:T}}fn wrap<T>(x:T)->E<T>{let w=Wrap{value:x};E::One{value:w.value}}fn run()->i32{let v=7;let e=wrap::<&i32>(&v);match e{E::One{value}=>*value}}\nfn main(){if run()==7{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 7,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "73cfe29f0262d4cbec5f897dc96118d88bdd4a07f32cdea4bceade061441a530",
    "full_source_sha256": "e6d89f8064acb47085654cd52ad51a0bce2e4f2d0303fe60eaaee7dd175fe0f6",
    "source_bytes": 186,
    "full_source_bytes": 251,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P08",
    "name": "partial_component_move_preserves_sibling_carriers",
    "group": "positive",
    "proposed_group": "positive",
    "category": "typed subtree clearing",
    "status": "valid",
    "source": "fn second<T>(x:T,y:T)->T{let pair=(x,y);let first=pair.0;let second=pair.1;let _=first;second}fn run()->i32{let a=1;let b=8;let r=second::<&i32>(&a,&b);*r}",
    "full_source": "fn second<T>(x:T,y:T)->T{let pair=(x,y);let first=pair.0;let second=pair.1;let _=first;second}fn run()->i32{let a=1;let b=8;let r=second::<&i32>(&a,&b);*r}\nfn main(){if run()==8{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 8,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "fca6e0bbea323d5491c795f04fe0712a06d43060727db4a104fa7fa6db51efa4",
    "full_source_sha256": "124831f5161fd0671cdab3d29b7aceef92d453624cc775f52826a2d43fecb2f7",
    "source_bytes": 155,
    "full_source_bytes": 220,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P09",
    "name": "shared_carrier_put_replacement_preserves_incoming_origin",
    "group": "positive",
    "proposed_group": "positive",
    "category": "full cells and put",
    "status": "valid",
    "source": "fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let a=1;let b=9;let mut slot=&a;put::<&i32>(&mut slot,&b);*slot}",
    "full_source": "fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let a=1;let b=9;let mut slot=&a;put::<&i32>(&mut slot,&b);*slot}\nfn main(){if run()==9{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 9,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "bdd0dbdb91d1041574987166f4ef95e764e69e381725c6c74db648ced4d6e5b7",
    "full_source_sha256": "e2bac5b8ab761b2e36225493cfbb6ebb47aab68dcd5f078c9b730c2802df6869",
    "source_bytes": 112,
    "full_source_bytes": 177,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P10",
    "name": "single_affine_put_preserves_checked_child_dependencies",
    "group": "positive",
    "proposed_group": "positive",
    "category": "one affine write",
    "status": "valid",
    "source": "fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let mut a=1;let mut b=2;let mut slot=&mut a;put::<&mut i32>(&mut slot,&mut b);*slot=10;b}",
    "full_source": "fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let mut a=1;let mut b=2;let mut slot=&mut a;put::<&mut i32>(&mut slot,&mut b);*slot=10;b}\nfn main(){if run()==10{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 10,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "8cd777dda29fa03cea1448c6fe53e6c2bb172bdd67dcaf491e8af2e8e72abb3d",
    "full_source_sha256": "4c43fb07b4b421fba1ed5bb63a75101b591b8695b288b0bc180de916a945b08e",
    "source_bytes": 137,
    "full_source_bytes": 203,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P11",
    "name": "current_cell_reload_and_return_follow_written_poststate",
    "group": "positive",
    "proposed_group": "positive",
    "category": "current storage and poststate",
    "status": "valid",
    "source": "fn replace<T:Copy>(dst:&mut T,x:T)->T{*dst=x;*dst}fn run()->i32{let a=1;let b=11;let mut slot=&a;let out=replace::<&i32>(&mut slot,&b);let first=*out;let second=*slot;first*100+second}",
    "full_source": "fn replace<T:Copy>(dst:&mut T,x:T)->T{*dst=x;*dst}fn run()->i32{let a=1;let b=11;let mut slot=&a;let out=replace::<&i32>(&mut slot,&b);let first=*out;let second=*slot;first*100+second}\nfn main(){if run()==1111{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 1111,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "9f2b3838b1b7df3d50e43f7843c79ba70e1302b4bc8d8ee0fea26bc6cc570058",
    "full_source_sha256": "b9f2e3c478f5a65d7e91dd84315db10de36e996b7787343dfcf23426e2950136",
    "source_bytes": 184,
    "full_source_bytes": 252,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P12",
    "name": "derived_aliases_observe_one_current_canonical_cell",
    "group": "positive",
    "proposed_group": "positive",
    "category": "physical cell identity",
    "status": "valid",
    "source": "fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let a=1;let b=12;let mut slot=&a;let parent=&mut slot;{let child=&mut *parent;put::<&i32>(child,&b);}**parent}",
    "full_source": "fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let a=1;let b=12;let mut slot=&a;let parent=&mut slot;{let child=&mut *parent;put::<&i32>(child,&b);}**parent}\nfn main(){if run()==12{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 12,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "7fa59032bc00355e3ddaae27676b19f830ce7a59e3ea6a0c297adbb43c167b25",
    "full_source_sha256": "cb79143202eca3357da1f953e0f6cffb77b9178174e043ac62aad835dd220e5e",
    "source_bytes": 158,
    "full_source_bytes": 224,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P13",
    "name": "separate_call_occurrences_keep_fresh_lifetime_bindings",
    "group": "positive",
    "proposed_group": "positive",
    "category": "call freshness",
    "status": "valid",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let long=13;let a=id::<&i32>(&long);let first=*a;let second;{let short=14;let b=id::<&i32>(&short);second=*b;}first*100+second}",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let long=13;let a=id::<&i32>(&long);let first=*a;let second;{let short=14;let b=id::<&i32>(&short);second=*b;}first*100+second}\nfn main(){if run()==1314{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 1314,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "97a6ec0711fa295012d3385e906e79b3e8871b77a2c1ab90ba71d050d1f36d8f",
    "full_source_sha256": "7efd174200999f19773adf61d7a3c3c733b9fbd7a4f1347c0df9c3de52e15b86",
    "source_bytes": 160,
    "full_source_bytes": 228,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P14",
    "name": "shared_join_and_rebind_union_component_origins",
    "group": "positive",
    "proposed_group": "positive",
    "category": "joins rebind and lastuse",
    "status": "valid",
    "source": "fn choose<T>(flag:bool,x:T,y:T)->T{if flag{x}else{y}}fn run()->i32{let a=14;let b=15;let mut r=choose::<&i32>(true,&a,&b);let first=*r;r=choose::<&i32>(false,&a,&b);let second=*r;first*100+second}",
    "full_source": "fn choose<T>(flag:bool,x:T,y:T)->T{if flag{x}else{y}}fn run()->i32{let a=14;let b=15;let mut r=choose::<&i32>(true,&a,&b);let first=*r;r=choose::<&i32>(false,&a,&b);let second=*r;first*100+second}\nfn main(){if run()==1415{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 1415,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "6f2ec2a3ab885769b8e7bea4ef32849618a10db3b88f405d030ce9e1cc2d62a3",
    "full_source_sha256": "093f46ed66c7eb3b5afe78e4ce835c7ac27ac401c082bd090a2c294942990cd5",
    "source_bytes": 196,
    "full_source_bytes": 264,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P15",
    "name": "firm_explicit_mutable_expectation_retains_checked_reborrow",
    "group": "positive",
    "proposed_group": "positive",
    "category": "source adjustment",
    "status": "valid",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let mut v=1;let r=&mut v;{let child=id::<&mut i32>(r);*child=14;}*r=15;*r}",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let mut v=1;let r=&mut v;{let child=id::<&mut i32>(r);*child=14;}*r=15;*r}\nfn main(){if run()==15{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 15,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "7f85f3fae91a705c205567bb33829d89ebb1be7503b8f5c7a93ed24730558512",
    "full_source_sha256": "1d3519562ed4e1d929fffd5ff82d32af668d152ec7b7b58fd4eb5747b122af71",
    "source_bytes": 107,
    "full_source_bytes": 173,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P16",
    "name": "recursive_shared_forwarding_uses_declared_envelope",
    "group": "positive",
    "proposed_group": "positive",
    "category": "recursive bootstrap",
    "status": "valid",
    "source": "fn walk<T:Copy>(x:T,n:i32)->T{if n==0{x}else{walk::<T>(x,n-1)}}fn run()->i32{let v=16;let r=walk::<&i32>(&v,3);*r}",
    "full_source": "fn walk<T:Copy>(x:T,n:i32)->T{if n==0{x}else{walk::<T>(x,n-1)}}fn run()->i32{let v=16;let r=walk::<&i32>(&v,3);*r}\nfn main(){if run()==16{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 16,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "1def6256fc441d8956819516d368518894e6068cc1bb09c1cf435f594f000bd6",
    "full_source_sha256": "62a31c914b8b3ee867b27bc046829199e8970d8713513337f7130beeafa44e5c",
    "source_bytes": 114,
    "full_source_bytes": 180,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "N17",
    "name": "local_referent_escapes_through_opaque_return",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "return lifetime",
    "status": "invalid",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let out;{let local=17;out=id::<&i32>(&local);}*out}",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let out;{let local=17;out=id::<&i32>(&local);}*out}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "acbde04f31a3f5f6e081c2b631d06c24ceb2004536a1082591a616a7c9de73cb",
    "full_source_sha256": "1036ee1e61a7a76aa7a45a27f73605da8772f972f372d3f48edf9b39df11b02c",
    "source_bytes": 84,
    "full_source_bytes": 149,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0597"
    ],
    "stock_check_error_codes": [
      "E0597"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0597",
        "message": "`local` does not live long enough"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0597",
        "message": "`local` does not live long enough"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N18",
    "name": "short_incoming_reference_escapes_external_put_cell",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "store lifetime",
    "status": "invalid",
    "source": "fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let long=1;let mut slot=&long;{let short=18;put::<&i32>(&mut slot,&short);}*slot}",
    "full_source": "fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let long=1;let mut slot=&long;{let short=18;put::<&i32>(&mut slot,&short);}*slot}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "e706f41243d3f45ac8ff24b3099fc63cbea851460cd975253b8d7a43ea30ebe9",
    "full_source_sha256": "1bb5c4d56620d7bf285662428676e0fb1513075fbf004866ebf7766b0fd7e2c6",
    "source_bytes": 129,
    "full_source_bytes": 194,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0597"
    ],
    "stock_check_error_codes": [
      "E0597"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0597",
        "message": "`short` does not live long enough"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0597",
        "message": "`short` does not live long enough"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N19",
    "name": "mutable_hidden_component_region_invariance_violation",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "invariance",
    "status": "invalid",
    "source": "fn put<T>(dst:&mut T,x:T){*dst=x;}fn invariant<'a,'b>(dst:&mut &'a i32,x:&'b i32){put::<&'a i32>(dst,x);}fn run()->i32{0}",
    "full_source": "fn put<T>(dst:&mut T,x:T){*dst=x;}fn invariant<'a,'b>(dst:&mut &'a i32,x:&'b i32){put::<&'a i32>(dst,x);}fn run()->i32{0}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "0421f444bf2e109fd2aeb8f66a0c82444ba2c726088a17ed3852889ef08b8c0d",
    "full_source_sha256": "d1db30629f7f2d80681e9a066df19f30bf76c29722f52bb425c4d44abfe1c2ab",
    "source_bytes": 121,
    "full_source_bytes": 186,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": null,
        "message": "lifetime may not live long enough"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": null,
        "message": "lifetime may not live long enough"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N20",
    "name": "same_erased_type_reuses_unrelated_call_lifetime",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "call freshness denial",
    "status": "invalid",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let long=1;let out;{let short=20;let a=id::<&i32>(&long);let b=id::<&i32>(&short);out=b;let _=a;}*out}",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let long=1;let out;{let short=20;let a=id::<&i32>(&long);let b=id::<&i32>(&short);out=b;let _=a;}*out}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "fe40edb6837538af73a9fc6ddbe4e57f56cb742ad80be7b7631664552c03277a",
    "full_source_sha256": "fb97bf32e7979252b52f5c9cd526d445799c5a7cc1e65791e0d8ac8a661075a8",
    "source_bytes": 135,
    "full_source_bytes": 200,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0597"
    ],
    "stock_check_error_codes": [
      "E0597"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0597",
        "message": "`short` does not live long enough"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0597",
        "message": "`short` does not live long enough"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N21",
    "name": "distinct_parameter_occurrences_cannot_exchange_return_authority",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "T U authority",
    "status": "invalid",
    "source": "fn id<T>(x:T)->T{x}fn wrong<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a U{let _=x;id::<&'b U>(y)}fn run()->i32{let long=1;let out;{let short=21;out=wrong::<i32,i32>(&long,&short);}*out}",
    "full_source": "fn id<T>(x:T)->T{x}fn wrong<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a U{let _=x;id::<&'b U>(y)}fn run()->i32{let long=1;let out;{let short=21;out=wrong::<i32,i32>(&long,&short);}*out}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "480469fd43e8b5b94398016f54e5e8edd8450a07d8601c467d69ea48d7cbd197",
    "full_source_sha256": "8c55122ca2a5f66082114f90837bfd4cde7e9cdc031422b19dbd286fd211050f",
    "source_bytes": 174,
    "full_source_bytes": 239,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": null,
        "message": "lifetime may not live long enough"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": null,
        "message": "lifetime may not live long enough"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N22",
    "name": "affine_root_actual_is_used_after_explicit_move",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "Move consumption",
    "status": "invalid",
    "source": "fn id<T>(x:T)->T{x}fn bad<T>(x:T)->T{let first=id::<T>(x);let second=id::<T>(x);let _=first;second}fn run()->i32{let mut v=22;let r=bad::<&mut i32>(&mut v);*r}",
    "full_source": "fn id<T>(x:T)->T{x}fn bad<T>(x:T)->T{let first=id::<T>(x);let second=id::<T>(x);let _=first;second}fn run()->i32{let mut v=22;let r=bad::<&mut i32>(&mut v);*r}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "314e848a480bb77a027ea6a90911a7c731885e8a8c4754ba4e9dc5fd15e9d3e1",
    "full_source_sha256": "211dc78e797493b984b306149a1168ba45ae434b8f5dc4c907fa75df59d0e6a8",
    "source_bytes": 159,
    "full_source_bytes": 224,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0382"
    ],
    "stock_check_error_codes": [
      "E0382"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0382",
        "message": "use of moved value: `x`"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0382",
        "message": "use of moved value: `x`"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N23",
    "name": "unbounded_opaque_parameter_is_duplicated_without_copy",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "universal body Copy denial",
    "status": "invalid",
    "source": "fn duplicate<T>(x:T)->(T,T){(x,x)}fn run()->i32{let v=23;let p=duplicate::<&i32>(&v);*p.0+*p.1}",
    "full_source": "fn duplicate<T>(x:T)->(T,T){(x,x)}fn run()->i32{let v=23;let p=duplicate::<&i32>(&v);*p.0+*p.1}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "325eb64ac059f5d9a29567449409bdf1002713802d4beed5dfe1653fa932492a",
    "full_source_sha256": "688c7a003c2afcbeb6f24dab8d9116cd0aca5e01c2c31b7fd398041c21c01e44",
    "source_bytes": 95,
    "full_source_bytes": 160,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0382"
    ],
    "stock_check_error_codes": [
      "E0382"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0382",
        "message": "use of moved value: `x`"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0382",
        "message": "use of moved value: `x`"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N24",
    "name": "indirect_put_through_shared_handle_is_invalid",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "mutable authority",
    "status": "invalid",
    "source": "fn wrong<T>(dst:&T,x:T){*dst=x;}fn run()->i32{let v=24;let slot=&v;wrong::<&i32>(&slot,&v);0}",
    "full_source": "fn wrong<T>(dst:&T,x:T){*dst=x;}fn run()->i32{let v=24;let slot=&v;wrong::<&i32>(&slot,&v);0}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "fd3e7a837e33b610b6e11848d1ff0f8001a0ff8805a804ac666b3eedaf27cddc",
    "full_source_sha256": "fb970e5da8c42dcdebbce8e2251feb4ea48129ae6a1f40b0ae837c98cf5011fd",
    "source_bytes": 93,
    "full_source_bytes": 158,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0594"
    ],
    "stock_check_error_codes": [
      "E0594"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0594",
        "message": "cannot assign to `*dst`, which is behind a `&` reference"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0594",
        "message": "cannot assign to `*dst`, which is behind a `&` reference"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N25",
    "name": "live_derived_shared_alias_conflicts_with_mutable_cell_access",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "alias loan conflict",
    "status": "invalid",
    "source": "fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let a=1;let b=25;let mut slot=&a;let parent=&mut slot;let alias=&*parent;put::<&i32>(parent,&b);**alias}",
    "full_source": "fn put<T>(dst:&mut T,x:T){*dst=x;}fn run()->i32{let a=1;let b=25;let mut slot=&a;let parent=&mut slot;let alias=&*parent;put::<&i32>(parent,&b);**alias}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "0b74882b57f18ca5f46f5d9ca14bddac42926cca0581bb2fb44ada2cf5ab2cd3",
    "full_source_sha256": "d8a74e3e4ee298c34b8ac7b3407d4a703bb1950ade719c2ee4c42cb7c4b447a2",
    "source_bytes": 152,
    "full_source_bytes": 217,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0502"
    ],
    "stock_check_error_codes": [
      "E0502"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0502",
        "message": "cannot borrow `*parent` as mutable because it is also borrowed as immutable"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0502",
        "message": "cannot borrow `*parent` as mutable because it is also borrowed as immutable"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N26",
    "name": "stored_reborrow_child_does_not_reactivate_parent",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "outer unique parent lifetime and freeze",
    "status": "invalid",
    "source": "fn bad<'a,'b>(p:&'a mut &'b mut i32)->&'b mut i32{&mut **p}fn run()->i32{0}",
    "full_source": "fn bad<'a,'b>(p:&'a mut &'b mut i32)->&'b mut i32{&mut **p}fn run()->i32{0}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "95ec51aac2a0dfa4ef7cbcc5295cabb2d0c50f7b9d19c3da81f2c6fac11cd102",
    "full_source_sha256": "d3e34c78496d41ab139dd51423f17b64697b3c3d18a550018c804b9e3b46975d",
    "source_bytes": 75,
    "full_source_bytes": 140,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": null,
        "message": "lifetime may not live long enough"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": null,
        "message": "lifetime may not live long enough"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N27",
    "name": "returned_written_cell_cannot_hide_short_incoming_origin",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "poststate escape",
    "status": "invalid",
    "source": "fn replace<T:Copy>(dst:&mut T,x:T)->T{*dst=x;*dst}fn run()->i32{let long=1;let mut slot=&long;let out;{let short=27;out=replace::<&i32>(&mut slot,&short);}*out}",
    "full_source": "fn replace<T:Copy>(dst:&mut T,x:T)->T{*dst=x;*dst}fn run()->i32{let long=1;let mut slot=&long;let out;{let short=27;out=replace::<&i32>(&mut slot,&short);}*out}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "d38b32d26ec3d443ccc303390201457d10ed6656cbb7409c2d65dfc8431fe3ee",
    "full_source_sha256": "8802594922bb0dd60204aa43adc9cf84ac61864911d2fd0dac9f706f5740acbd",
    "source_bytes": 160,
    "full_source_bytes": 225,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0597"
    ],
    "stock_check_error_codes": [
      "E0597"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0597",
        "message": "`short` does not live long enough"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0597",
        "message": "`short` does not live long enough"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N28",
    "name": "unused_generic_body_has_invalid_carrier_escape",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "all body gate",
    "status": "invalid",
    "source": "fn bad<'a,T>(x:T)->&'a i32{let local=28;let _=x;&local}fn run()->i32{0}",
    "full_source": "fn bad<'a,T>(x:T)->&'a i32{let local=28;let _=x;&local}fn run()->i32{0}\nfn main(){if run()==0{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "2199bb237fcffb264ff98399a3b7cf88596962e39725fdbb2cc0b76015a32555",
    "full_source_sha256": "443f10d4a8a5fef503487c67590fb3bca04ab21da8778bd41eee401bbf1217ce",
    "source_bytes": 71,
    "full_source_bytes": 136,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0515"
    ],
    "stock_check_error_codes": [
      "E0515"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0515",
        "message": "cannot return reference to local variable `local`"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0515",
        "message": "cannot return reference to local variable `local`"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "X29",
    "name": "newly_inferred_structural_reference_actual",
    "group": "legal-excluded",
    "proposed_group": "legal-excluded",
    "category": "inference fence",
    "status": "valid",
    "source": "fn id<T>(x:T)->T{x}fn run()->i32{let v=29;let out=id(&v);*out}",
    "full_source": "fn id<T>(x:T)->T{x}fn run()->i32{let v=29;let out=id(&v);*out}\nfn main(){if run()==29{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 29,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "d3cb19f3247860db9317cdd9a019b63c7c076dae483faf56440a0ba1c39694e4",
    "full_source_sha256": "d2ae1aba38051258e57adbe598470b25022ab9a72f924d8aeadcd06f6afb6be8",
    "source_bytes": 62,
    "full_source_bytes": 128,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "X30",
    "name": "multi_expanded_affine_result_components",
    "group": "legal-excluded",
    "proposed_group": "legal-excluded",
    "category": "expanded affine bound",
    "status": "valid",
    "source": "fn pair<T,U>(x:T,y:U)->(T,U){(x,y)}fn run()->i32{let mut a=1;let mut b=2;let (x,y)=pair::<&mut i32,&mut i32>(&mut a,&mut b);*x=10;*y=20;a+b}",
    "full_source": "fn pair<T,U>(x:T,y:U)->(T,U){(x,y)}fn run()->i32{let mut a=1;let mut b=2;let (x,y)=pair::<&mut i32,&mut i32>(&mut a,&mut b);*x=10;*y=20;a+b}\nfn main(){if run()==30{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 30,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "1c3b169441a3f2bb96f9d0887782c51c7e74843f0f2d4fc4b37df235ea1e67ff",
    "full_source_sha256": "5dd398d15b324afe1e89fe42eaa3b4b67cd8ee62e2ac4c388b9f81a61033cdb5",
    "source_bytes": 140,
    "full_source_bytes": 206,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "X31",
    "name": "mixed_affine_result_and_written_cell_flows",
    "group": "legal-excluded",
    "proposed_group": "legal-excluded",
    "category": "correlation boundary",
    "status": "valid",
    "source": "fn put_and_view<'a,T>(dst:&'a mut T,x:T)->&'a mut T{*dst=x;dst}fn run()->i32{let mut a=1;let mut b=2;let mut slot=&mut a;let r=put_and_view::<&mut i32>(&mut slot,&mut b);**r=31;b}",
    "full_source": "fn put_and_view<'a,T>(dst:&'a mut T,x:T)->&'a mut T{*dst=x;dst}fn run()->i32{let mut a=1;let mut b=2;let mut slot=&mut a;let r=put_and_view::<&mut i32>(&mut slot,&mut b);**r=31;b}\nfn main(){if run()==31{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 31,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "e156d4741f4792417dee17a59fe195cf672377d512dfa63ee49e1de2933f663b",
    "full_source_sha256": "82f296a16e53b5618f5f9c6c3cf1c4bc7f5617c37e36646a2d7d308194ce9ada",
    "source_bytes": 179,
    "full_source_bytes": 245,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "X32",
    "name": "derived_internal_nominal_reference_actual_call",
    "group": "legal-excluded",
    "proposed_group": "legal-excluded",
    "category": "derived nominal reference executable actual fence",
    "status": "valid",
    "source": "struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn outer<T>(x:T)->T{let wrapped=Wrap{value:x};let out=id::<Wrap<T>>(wrapped);out.value}fn run()->i32{let v=32;let r=outer::<&i32>(&v);*r}",
    "full_source": "struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn outer<T>(x:T)->T{let wrapped=Wrap{value:x};let out=id::<Wrap<T>>(wrapped);out.value}fn run()->i32{let v=32;let r=outer::<&i32>(&v);*r}\nfn main(){if run()==32{println!(\"generic carriers oracle ok\");}}\n",
    "expected": 32,
    "expected_stdout": "generic carriers oracle ok\n",
    "classification_observed": true,
    "source_sha256": "09e40d1c8632afb2dcc97e16c95332fe154d57327eb5822069fe3971afe40ae1",
    "full_source_sha256": "673a1575da3e755d9529c4c3f5522a0a661c24fd6e7e8753213c166cbb9eae29",
    "source_bytes": 179,
    "full_source_bytes": 245,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "generic carriers oracle ok\n",
    "stock_runtime_stderr": ""
  }
]
'''
FIXTURES = json.loads(FROZEN_CORPUS_JSON)

def corpus():
    if hashlib.sha256(FROZEN_CORPUS_JSON.encode()).hexdigest() != ACCEPTANCE_CORPUS_SHA256:
        raise ValueError("frozen generic carrier dictionary bytes changed")
    kinds = {"positive": "valid", "invalid": "invalid", "legal-excluded": "boundary"}
    if len(FIXTURES) != 32 or len({f["name"] for f in FIXTURES}) != 32:
        raise ValueError("expected exactly32 unique generic carrier fixtures")
    if [sum(f["group"] == g for f in FIXTURES) for g in kinds] != [16, 12, 4]:
        raise ValueError("frozen generic carrier classifications changed")
    for fixture in FIXTURES:
        source = fixture["full_source"]
        if (hashlib.sha256(source.encode()).hexdigest() != fixture["full_source_sha256"]
                or len(source.encode()) != fixture["full_source_bytes"]
                or hashlib.sha256(fixture["source"].encode()).hexdigest() != fixture["source_sha256"]
                or len(fixture["source"].encode()) != fixture["source_bytes"]):
            raise ValueError(f"frozen source changed: {fixture['name']}")
        yield Case(fixture["name"], kinds[fixture["group"]], source,
                   fixture["expected_stdout"])


def error_diagnostics(receipt):
    """Retain exact primary level/code/message multiplicities, including no-code errors."""
    diagnostics = []
    malformed = []
    for line in receipt["stderr"].splitlines():
        try:
            diagnostic = json.loads(line)
        except json.JSONDecodeError:
            malformed.append(line)
            continue
        if not isinstance(diagnostic, dict):
            malformed.append(line)
            continue
        if diagnostic.get("level") != "error":
            continue
        message = diagnostic.get("message")
        if isinstance(message, str) and message.startswith("aborting due to"):
            continue
        code = diagnostic.get("code")
        if code is not None and (not isinstance(code, dict)
                                 or not isinstance(code.get("code"), str)):
            malformed.append(line)
            continue
        diagnostics.append({"level": "error", "code": code["code"] if code else None,
                            "message": message})
    return diagnostics, malformed


def artifact_metadata(path):
    """Record real artifact identity, including genuine zero-byte bin metadata."""
    if not path.is_file():
        return None
    return {"path": str(path), "bytes": path.stat().st_size,
            "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def stock_case(case, index, directory, rustc, compile_timeout, run_timeout):
    """Independent real BUILD plus metadata-only CHECK, never metadata+link."""
    directory.mkdir()
    source = directory / "input.rs"
    source.write_text(case.source, encoding="utf-8")
    output = directory / "stock"
    metadata = directory / "stock.rmeta"
    argv = [rustc, "--edition=2024", "--crate-name", "reference", "-A", "warnings",
            "-D", "overflowing_literals", "-D", "arithmetic_overflow", "-C",
            "overflow-checks=yes", "--error-format=json", source]
    result = process([*argv, "--emit=link", "-o", output], directory, compile_timeout)
    check = process([*argv, "--emit=metadata", "-o", metadata], directory, compile_timeout)
    fixture = FIXTURES[index]
    build_errors, build_malformed = error_diagnostics(result)
    check_errors, check_malformed = error_diagnostics(check)
    row = {"record": "stock_case", "index": index, "name": case.name,
           "kind": case.kind, "source": case.source,
           "source_sha256": hashlib.sha256(case.source.encode()).hexdigest(),
           "stock_compile": result, "stock_check": check,
           "stock_compile_argv": [str(arg) for arg in [*argv, "--emit=link", "-o", output]],
           "stock_check_argv": [str(arg) for arg in [*argv, "--emit=metadata", "-o", metadata]],
           "stock_compile_artifact_metadata": artifact_metadata(output),
           "stock_check_artifact_metadata": artifact_metadata(metadata),
           "stock_compile_artifact": output.is_file(),
           "stock_check_artifact": metadata.is_file(),
           "stock_error_diagnostics": build_errors,
           "stock_check_error_diagnostics": check_errors,
           "stock_error_codes": [d["code"] for d in build_errors if d["code"]],
           "stock_check_error_codes": [d["code"] for d in check_errors if d["code"]],
           "failures": []}
    if build_malformed or check_malformed or result["stdout"] or check["stdout"]:
        row["failures"].append("stock compile diagnostics must be structured JSON on stderr only")
    if (build_errors != fixture["stock_error_diagnostics"]
            or check_errors != fixture["stock_check_error_diagnostics"]
            or row["stock_error_codes"] != fixture["stock_error_codes"]
            or row["stock_check_error_codes"] != fixture["stock_check_error_codes"]):
        row["failures"].append("stock primary diagnostic code/message multiplicities differ from freeze")
    if case.kind == "invalid":
        if (not rejected(result) or output.exists() or not rejected(check)
                or metadata.exists() or not build_errors or not check_errors):
            row["failures"].append("invalid fixture must reject both stock phases with exit1 and no artifacts")
        if fixture["id"] in ("N19", "N21", "N26"):
            required = [{"level": "error", "code": None,
                         "message": "lifetime may not live long enough"}]
            if build_errors != required or check_errors != required:
                row["failures"].append("N19/N21/N26 require exact structured code-less lifetime diagnostic")
    elif (not accepted(result) or not output.is_file()
          or not accepted(check) or not metadata.is_file()):
        row["failures"].append("valid/boundary fixture must retain BUILD executable and actual CHECK metadata")
    else:
        row["stock_run"] = process([output], directory, run_timeout)
        run = row["stock_run"]
        if not accepted(run) or run["stdout"] != case.stdout or run["stderr"]:
            row["failures"].append("stock runtime differs from exact stdout/empty stderr/zero exit")
    row["passed"] = not row["failures"]
    row["outcome"] = "stock_classified" if row["passed"] else "failure"
    return row


def native_case(case, index, directory, binary, rustc, compile_timeout, run_timeout):
    """Use supplied native binary and shared bounded worker1/8 BUILD/CHECK/runtime."""
    row = run_case(case, index, directory, binary, rustc, compile_timeout, run_timeout)
    if case.kind in ("invalid", "boundary"):
        for worker in row["workers"]:
            for mode in ("compile", "check"):
                text = worker[mode]["stderr"].lower()
                if any(marker in text for marker in ("panic", "parse", "unexpected")):
                    row["failures"].append("rejection must not be a parser failure, panic or unexpected error")
                unsupported = "unsupported" in text or "not supported" in text
                if case.kind == "invalid" and unsupported:
                    row["failures"].append("invalid fixture requires semantic rejection, not unsupported failure")
                if case.kind == "boundary" and not unsupported:
                    row["failures"].append("legal-excluded fixture requires an explicit unsupported boundary")
    row["passed"] = not row["failures"]
    if not row["passed"]:
        row["outcome"] = "failure"
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
    rustc = args.rustc.absolute()
    if not rustc.is_file():
        parser.error("--rustc must name an existing executable")
    binary = args.binary.resolve(strict=True) if args.binary else None
    cases = list(corpus())
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-generic-carriers-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "generic-carriers-v1",
                "case_count": len(cases),
                "case_counts": {kind: sum(c.kind == kind for c in cases)
                                for kind in ("valid", "invalid", "boundary")},
                "stock_only": args.stock_only,
                "workers": [] if args.stock_only else [1, 8],
                "modes": [] if args.stock_only else ["build", "check"], "edition": 2024,
                "reference_modes": ["build", "check", "run"] if args.stock_only else ["build", "run"],
                "rustc_path": str(rustc), "reference": version["stdout"],
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest() if binary else None,
                "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                "shared_harness_sha256": hashlib.sha256(Path(__file__).with_name("module_correctness.py").read_bytes()).hexdigest(),
                "corpus_sha256": corpus_hash,
                "acceptance_corpus_sha256": ACCEPTANCE_CORPUS_SHA256,
                "timing_scope": "process/deadline/capture overhead included, not a compiler latency benchmark",
                "note": "legal excluded boundaries are not invalid Rust or native parity"})
        totals = {"accepted_parity": 0, "invalid_rejected": 0, "unsupported_boundary": 0,
                  "stock_classified": 0, "failure": 0}
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
            if not row["passed"]:
                record({"record": "summary", **totals, "passed": False})
                raise SystemExit(f"generic carrier discrepancy, frozen sources not changed: {args.output}")
        record({"record": "summary", **totals, "passed": totals["failure"] == 0})


if __name__ == "__main__":
    main()
