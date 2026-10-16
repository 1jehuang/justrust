"""Frozen exact32 canonical Copy/Clone differential corpus.

Sixteen positive, twelve invalid, four legal-excluded unchanged stock fixtures.
Uses shared bounded runner with a supplied native binary, never builds it.
Stock CHECK is metadata-only and accepts actual regular zero-byte bin rmeta.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case

ACCEPTANCE_CORPUS_SHA256 = "3c3759705d094af4f4c53a53e8bcb3bc400da7e1d31a84e878f5e2708502b1d7"

# Exact observed dictionaries with distinct declaration and full-source hash domains.
FROZEN_CORPUS_JSON = r'''[
  {
    "id": "P01",
    "name": "canonical_core_std_identity_and_shadowable_fake_names",
    "group": "positive",
    "proposed_group": "positive",
    "category": "canonical identity and shadowing",
    "status": "valid",
    "source": "trait Copy{fn fake(&self)->i32;}trait Clone{fn fake_clone(&self)->i32;}struct S{v:i32}impl core::clone::Clone for S{fn clone(&self)->Self{S{v:self.v}}}impl core::marker::Copy for S{}fn twice<T:std::marker::Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let s=S{v:3};let p=twice::<S>(s);p.0.v+p.1.v}",
    "full_source": "trait Copy{fn fake(&self)->i32;}trait Clone{fn fake_clone(&self)->i32;}struct S{v:i32}impl core::clone::Clone for S{fn clone(&self)->Self{S{v:self.v}}}impl core::marker::Copy for S{}fn twice<T:std::marker::Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let s=S{v:3};let p=twice::<S>(s);p.0.v+p.1.v}\nfn main(){if run()==6{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 6,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "874f98bbb9cea3bce5fb93b1cb94c9e8fa8297390655320183683ff201a14625",
    "full_source_sha256": "d6afb365be169612592dcb2abea82ab08860543017df05235f1e790eed09dae7",
    "source_bytes": 289,
    "full_source_bytes": 352,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P02",
    "name": "true_supertrait_closure_and_default_sized_generic",
    "group": "positive",
    "proposed_group": "positive",
    "category": "general supertraits and default Sized",
    "status": "valid",
    "source": "trait Base{fn base(&self)->i32;}trait Child:Base{fn child(&self)->i32;}struct S{v:i32}impl Base for S{fn base(&self)->i32{self.v}}impl Child for S{fn child(&self)->i32{self.v+1}}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}}impl Copy for S{}fn via<T:Child>(x:&T)->i32{x.base()+x.child()}fn duplicate<T:Copy>(x:&T)->T{x.clone()}fn sized<T>(x:T)->T{x}fn run()->i32{let s=S{v:3};let c=duplicate::<S>(&s);c.v+via::<S>(&c)+sized::<i32>(1)}",
    "full_source": "trait Base{fn base(&self)->i32;}trait Child:Base{fn child(&self)->i32;}struct S{v:i32}impl Base for S{fn base(&self)->i32{self.v}}impl Child for S{fn child(&self)->i32{self.v+1}}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}}impl Copy for S{}fn via<T:Child>(x:&T)->i32{x.base()+x.child()}fn duplicate<T:Copy>(x:&T)->T{x.clone()}fn sized<T>(x:T)->T{x}fn run()->i32{let s=S{v:3};let c=duplicate::<S>(&s);c.v+via::<S>(&c)+sized::<i32>(1)}\nfn main(){if run()==11{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 11,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "b2c3e747b8b1384b555d4e723f4b9fe4b69e0f6b232ae1ba91ae12045f29e040",
    "full_source_sha256": "497b71de2585a7872bab7b7387c6e47de47453010e0825a56048797b37fce8be",
    "source_bytes": 440,
    "full_source_bytes": 504,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P03",
    "name": "manual_copy_clone_pair_body_uses_copy_without_header_cycle",
    "group": "positive",
    "proposed_group": "positive",
    "category": "independent Copy WF and Clone body",
    "status": "valid",
    "source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{*self}}impl Copy for S{}fn run()->i32{let s=S{v:7};let a=s;let b=s.clone();a.v+b.v}",
    "full_source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{*self}}impl Copy for S{}fn run()->i32{let s=S{v:7};let a=s;let b=s.clone();a.v+b.v}\nfn main(){if run()==14{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 14,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "3f811619fd070d9abbbf9a1e93efb40ac28645f43a146a9200c35766f6ad10f1",
    "full_source_sha256": "99573d6f76cda2ff6f69c0fcf3a3fff8c62102fb35f8685998f3bbf343dea014",
    "source_bytes": 137,
    "full_source_bytes": 201,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P04",
    "name": "conditional_nominal_copy_uses_only_declared_field_bounds",
    "group": "positive",
    "proposed_group": "positive",
    "category": "conditional explicit nominal Copy",
    "status": "valid",
    "source": "struct Wrap<T>{v:T}impl<T:Copy> Clone for Wrap<T>{fn clone(&self)->Self{*self}}impl<T:Copy> Copy for Wrap<T>{}fn twice<T:Copy>(x:Wrap<T>)->(Wrap<T>,Wrap<T>){(x,x)}fn run()->i32{let p=twice::<i32>(Wrap{v:8});p.0.v+p.1.v}",
    "full_source": "struct Wrap<T>{v:T}impl<T:Copy> Clone for Wrap<T>{fn clone(&self)->Self{*self}}impl<T:Copy> Copy for Wrap<T>{}fn twice<T:Copy>(x:Wrap<T>)->(Wrap<T>,Wrap<T>){(x,x)}fn run()->i32{let p=twice::<i32>(Wrap{v:8});p.0.v+p.1.v}\nfn main(){if run()==16{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 16,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "c48140519fe4b0d9c3db6e2ae97f135be0e98fccea08ad56206bfbf1536f471e",
    "full_source_sha256": "b7eadce86d9346e4e7e35521d7e60cb146c748bb63ebfb8c364ba2548dc40954",
    "source_bytes": 219,
    "full_source_bytes": 283,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P05",
    "name": "shared_reference_copy_without_referent_copy_or_clone",
    "group": "positive",
    "proposed_group": "positive",
    "category": "shared carrier evidence without referent traits",
    "status": "valid",
    "source": "struct Opaque{v:i32}fn run()->i32{let owner=Opaque{v:9};let r=&owner;let a=r;let b=r;a.v+b.v}",
    "full_source": "struct Opaque{v:i32}fn run()->i32{let owner=Opaque{v:9};let r=&owner;let a=r;let b=r;a.v+b.v}\nfn main(){if run()==18{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 18,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "474b5df2824be8bbd3df09027355577675a8422429bf15f055ef1429f155e165",
    "full_source_sha256": "a8c3163e81a657a8550fb2c33ff13c9615c7cb60535281f6538f340bfda2f9b6",
    "source_bytes": 93,
    "full_source_bytes": 157,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P06",
    "name": "primitive_unit_and_empty_product_clone_recipes",
    "group": "positive",
    "proposed_group": "positive",
    "category": "primitive bool unit and zero product bodies",
    "status": "valid",
    "source": "fn run()->i32{let n=10i32.clone();let yes=true.clone();let unit=().clone();let empty:[i32;0]=[];let cloned=empty.clone();let unit_again=unit.clone();if yes{n}else{0}}",
    "full_source": "fn run()->i32{let n=10i32.clone();let yes=true.clone();let unit=().clone();let empty:[i32;0]=[];let cloned=empty.clone();let unit_again=unit.clone();if yes{n}else{0}}\nfn main(){if run()==10{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 10,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "0f0165a502b8a5ae64e736c3a2a67fcf5ebeab6bc7432da44145f8be6f0ae09f",
    "full_source_sha256": "7c3af546df69b835e22526187e215e60a1661d4b4072d9cce900f05e60eff540",
    "source_bytes": 166,
    "full_source_bytes": 230,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P07",
    "name": "manual_clone_on_copy_nominal_preserves_observable_behavior",
    "group": "positive",
    "proposed_group": "positive",
    "category": "actual nontrivial source Clone on Copy",
    "status": "valid",
    "source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+5}}}impl Copy for S{}fn run()->i32{let s=S{v:1};let cloned=s.clone();let before=s.v;let after=cloned.v;before*100+after}",
    "full_source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+5}}}impl Copy for S{}fn run()->i32{let s=S{v:1};let cloned=s.clone();let before=s.v;let after=cloned.v;before*100+after}\nfn main(){if run()==106{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 106,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "209439262ba12a3c35b7b0583f142e5cda10c0de58d6b1d2f6b18a922b66f01b",
    "full_source_sha256": "eff42e7a0eb0198382f7fae0908d3e8f7de2508f1f8766dcc9c7ba284bda3bc0",
    "source_bytes": 185,
    "full_source_bytes": 250,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P08",
    "name": "tuple_clone_preserves_nontrivial_manual_copy_child_clone",
    "group": "positive",
    "proposed_group": "positive",
    "category": "tuple manual Copy child observations hypothesis",
    "status": "valid",
    "source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+1}}}impl Copy for S{}fn run()->i32{let pair=(S{v:1},S{v:2});let cloned=pair.clone();let first=cloned.0.v;let second=cloned.1.v;first*100+second}",
    "full_source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+1}}}impl Copy for S{}fn run()->i32{let pair=(S{v:1},S{v:2});let cloned=pair.clone();let first=cloned.0.v;let second=cloned.1.v;first*100+second}\nfn main(){if run()==203{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 203,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "548669d0402a3219237124137de3a861ab0fe03fd1b11219178d35061f4d97f4",
    "full_source_sha256": "5a69a379c35b110310acbbe645fc63c193cebcbd02cddc7124eeda54c8843cc6",
    "source_bytes": 209,
    "full_source_bytes": 274,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P09",
    "name": "array_clone_preserves_nontrivial_manual_copy_child_clone",
    "group": "positive",
    "proposed_group": "positive",
    "category": "array manual Copy child observations hypothesis",
    "status": "valid",
    "source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+1}}}impl Copy for S{}fn run()->i32{let array=[S{v:3},S{v:4}];let cloned=array.clone();let first=cloned[0].v;let second=cloned[1].v;first*100+second}",
    "full_source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+1}}}impl Copy for S{}fn run()->i32{let array=[S{v:3},S{v:4}];let cloned=array.clone();let first=cloned[0].v;let second=cloned[1].v;first*100+second}\nfn main(){if run()==405{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 405,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "d2754623697ca10bfe2cbca425e0a91527c809fe396a758bc855135d5bd04863",
    "full_source_sha256": "27793ab3d82b584051969044474530bb33c38cb23b60a5e571236031542dfe60",
    "source_bytes": 213,
    "full_source_bytes": 278,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P10",
    "name": "shared_reference_clone_inner_escapes_outer_handle",
    "group": "positive",
    "proposed_group": "positive",
    "category": "shared reference clone original inner provenance",
    "status": "valid",
    "source": "struct Opaque{v:i32}fn run()->i32{let owner=Opaque{v:11};let result;{let r=&owner;result=<&Opaque as core::clone::Clone>::clone(&r);}result.v}",
    "full_source": "struct Opaque{v:i32}fn run()->i32{let owner=Opaque{v:11};let result;{let r=&owner;result=<&Opaque as core::clone::Clone>::clone(&r);}result.v}\nfn main(){if run()==11{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 11,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "8f0799321aea9e3d74fba916565a7e4fb8c834c28079f8db6075153ded4c3661",
    "full_source_sha256": "10f64b520fb09f4cb4ee00ba8e300f44e6876f53a8f7539eef309cade06b6587",
    "source_bytes": 142,
    "full_source_bytes": 206,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P11",
    "name": "receiver_candidate_pointee_clone_precedes_reference_clone",
    "group": "positive",
    "proposed_group": "positive",
    "category": "ordered candidate pointee first",
    "status": "valid",
    "source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+1}}}fn run()->i32{let s=S{v:12};let r=&s;let cloned=r.clone();let original=s.v;let value=cloned.v;original*100+value}",
    "full_source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v+1}}}fn run()->i32{let s=S{v:12};let r=&s;let cloned=r.clone();let original=s.v;let value=cloned.v;original*100+value}\nfn main(){if run()==1213{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 1213,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "f67fefdc658e798a78e0e00476976695bad558d104476f7a28cd0e9030369979",
    "full_source_sha256": "357cd51ab4f0cd9e1741ec81d737b49ab7a9b19da5056b3f9337852127070354",
    "source_bytes": 182,
    "full_source_bytes": 248,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P12",
    "name": "receiver_candidate_shared_reference_fallback_and_inherent_priority",
    "group": "positive",
    "proposed_group": "positive",
    "category": "reference fallback and same-candidate inherent priority",
    "status": "valid",
    "source": "struct Opaque{v:i32}struct S{v:i32}impl S{fn clone(&self)->i32{self.v+10}}impl Clone for S{fn clone(&self)->Self{S{v:self.v+100}}}fn run()->i32{let owner=Opaque{v:13};let r=&owner;let cloned=r.clone();let first=cloned.v;let s=S{v:2};let second=s.clone();first*100+second}",
    "full_source": "struct Opaque{v:i32}struct S{v:i32}impl S{fn clone(&self)->i32{self.v+10}}impl Clone for S{fn clone(&self)->Self{S{v:self.v+100}}}fn run()->i32{let owner=Opaque{v:13};let r=&owner;let cloned=r.clone();let first=cloned.v;let s=S{v:2};let second=s.clone();first*100+second}\nfn main(){if run()==1312{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 1312,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "d83fbcce37ccc17cded20bc6aa3f376f7aeec1c99d4d38a936e63d0b4699b7cb",
    "full_source_sha256": "83cc73faff837bb455a6fbd304ccf13bad48f7f8ee7e8042936f4a568827e648",
    "source_bytes": 271,
    "full_source_bytes": 337,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P13",
    "name": "real_clone_from_override_runs_while_default_stays_unavailable",
    "group": "positive",
    "proposed_group": "positive",
    "category": "genuine supported source clone_from override",
    "status": "valid",
    "source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}fn clone_from(&mut self,source:&Self){self.v=source.v+10;}}fn run()->i32{let mut destination=S{v:1};let source=S{v:2};destination.clone_from(&source);destination.v}",
    "full_source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}fn clone_from(&mut self,source:&Self){self.v=source.v+10;}}fn run()->i32{let mut destination=S{v:1};let source=S{v:2};destination.clone_from(&source);destination.v}\nfn main(){if run()==12{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 12,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "f7ac913d8155dc2c4dd267be3ed5ae5c63d9ddf124f0f4e4befbadd74f61364f",
    "full_source_sha256": "c48601b0d9faa052bcff8e100bd43cfe1518d723562649dcf183989d34d2e881",
    "source_bytes": 230,
    "full_source_bytes": 294,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P14",
    "name": "logical_copy_clone_double_shared_reference_without_method_execution",
    "group": "positive",
    "proposed_group": "positive",
    "category": "logical double reference Copy with Clone supergoal",
    "status": "valid",
    "source": "fn run()->i32{let v=7;let r=&v;let rr=&r;let a=rr;let b=rr;**a+**b}",
    "full_source": "fn run()->i32{let v=7;let r=&v;let rr=&r;let a=rr;let b=rr;**a+**b}\nfn main(){if run()==14{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 14,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "1a3571936a1faca776ce8b9f4ae45c6c3ffd3c049185d12ad3e6332476ccd1df",
    "full_source_sha256": "42e90684aad86b489b7a7efd2291bbebe4c926f087fbcb4d2af96656ceef5d2c",
    "source_bytes": 67,
    "full_source_bytes": 131,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P15",
    "name": "T_U_coincidence_preserves_declared_clone_result_origins",
    "group": "positive",
    "proposed_group": "positive",
    "category": "known open RefSelf topology and declared origins",
    "status": "valid",
    "source": "fn keep<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a T{let ignored=y;<&T as core::clone::Clone>::clone(&x)}fn run()->i32{let long=17;let out;{let short=2;out=keep::<i32,i32>(&long,&short);}*out}",
    "full_source": "fn keep<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a T{let ignored=y;<&T as core::clone::Clone>::clone(&x)}fn run()->i32{let long=17;let out;{let short=2;out=keep::<i32,i32>(&long,&short);}*out}\nfn main(){if run()==17{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 17,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "8c39ac46818ced73cbe066bfb4b84bb71f4336da43d6e0891f5e8cb7e4c29b0e",
    "full_source_sha256": "681b706257171f38c63379529597d6fa38d1732120d47333af8c4dc5e022009a",
    "source_bytes": 182,
    "full_source_bytes": 246,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P16",
    "name": "unused_nominal_field_tuple33_array4096_does_not_poison_scalar_main",
    "group": "positive",
    "proposed_group": "positive",
    "category": "unused source shapes independent of recipe admission",
    "status": "valid",
    "source": "struct UnusedLarge{tuple:(i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32),array:[i32;4096]}fn run()->i32{19}",
    "full_source": "struct UnusedLarge{tuple:(i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32,i32),array:[i32;4096]}fn run()->i32{19}\nfn main(){if run()==19{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 19,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "f244a6bc638af5146c72fe8986162d359880db91064f8ceeb8f54ab2409fb5cf",
    "full_source_sha256": "23a611f242f2187cc4b6a8e01132862a132853707ba9e17067dab361ed9d2f09",
    "source_bytes": 193,
    "full_source_bytes": 257,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "N17",
    "name": "fake_copy_spelling_does_not_grant_second_owned_use",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "fake trait identity cannot grant Copy",
    "status": "invalid",
    "source": "trait Copy{fn tag(&self)->i32;}struct S{v:i32}impl Copy for S{fn tag(&self)->i32{self.v}}fn run()->i32{let s=S{v:1};let a=s;let b=s;a.v+b.v}",
    "full_source": "trait Copy{fn tag(&self)->i32;}struct S{v:i32}impl Copy for S{fn tag(&self)->i32{self.v}}fn run()->i32{let s=S{v:1};let a=s;let b=s;a.v+b.v}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "2ddf3302dbb4432781e9d28bd4a3a0aef1792875444cc0d4d9d09c2ccfd0c550",
    "full_source_sha256": "5f94d67f6d51ab8a186aac2eb8966157147d4408c3106fb8c6811beb26346a03",
    "source_bytes": 140,
    "full_source_bytes": 203,
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
        "message": "use of moved value: `s`"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0382",
        "message": "use of moved value: `s`"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N18",
    "name": "copy_shaped_nominal_without_copy_impl_remains_move_only",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "no structural nominal Copy permission",
    "status": "invalid",
    "source": "struct S{v:i32}fn run()->i32{let s=S{v:1};let a=s;let b=s;a.v+b.v}",
    "full_source": "struct S{v:i32}fn run()->i32{let s=S{v:1};let a=s;let b=s;a.v+b.v}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "bc5481638576b986847ed2da94f52ae7d7402b702b134eb0bee2ac9f97e652ca",
    "full_source_sha256": "8184804dbc0485aab9fd272cfaa57f5cfc930c2ce381142afb911c7f5f82fb07",
    "source_bytes": 66,
    "full_source_bytes": 129,
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
        "message": "use of moved value: `s`"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0382",
        "message": "use of moved value: `s`"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N19",
    "name": "conditional_copy_impl_missing_required_field_bound",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "missing owned field Copy obligation",
    "status": "invalid",
    "source": "struct S<T>{v:T}impl<T:Clone> Clone for S<T>{fn clone(&self)->Self{S{v:self.v.clone()}}}impl<T:Clone> Copy for S<T>{}fn run()->i32{0}",
    "full_source": "struct S<T>{v:T}impl<T:Clone> Clone for S<T>{fn clone(&self)->Self{S{v:self.v.clone()}}}impl<T:Clone> Copy for S<T>{}fn run()->i32{0}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "6ec796b8f063a3b2bdb60f8e97076eed69f71b46f4a2caa1ca7f2f962634c61e",
    "full_source_sha256": "d249ef174c52e4224b967375fe8ca148d5a0cbf4910a6b94c1ac0df61a9280c1",
    "source_bytes": 133,
    "full_source_bytes": 196,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0204"
    ],
    "stock_check_error_codes": [
      "E0204"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0204",
        "message": "the trait `Copy` cannot be implemented for this type"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0204",
        "message": "the trait `Copy` cannot be implemented for this type"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N20",
    "name": "copy_impl_without_clone_supertrait_is_invalid",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "missing real Clone supertrait",
    "status": "invalid",
    "source": "struct S{v:i32}impl Copy for S{}fn run()->i32{0}",
    "full_source": "struct S{v:i32}impl Copy for S{}fn run()->i32{0}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "5d2a58855f0db241a49f880d6b9be31b839b4bc0003d90311b4d69ab621e2495",
    "full_source_sha256": "818ad0226a8d569f1d6b23f0d58e55241693ff62231a93384471883d521e2e00",
    "source_bytes": 48,
    "full_source_bytes": 111,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0277"
    ],
    "stock_check_error_codes": [
      "E0277"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0277",
        "message": "the trait bound `S: Clone` is not satisfied"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0277",
        "message": "the trait bound `S: Clone` is not satisfied"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N21",
    "name": "unused_bad_clone_body_moves_noncopy_through_shared_self",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "unused Clone body move through shared self",
    "status": "invalid",
    "source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{*self}}fn run()->i32{0}",
    "full_source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{*self}}fn run()->i32{0}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "b56a995752409d1a6d99efe116b0996569e897d53b9d0a009e8d590bdb581e8f",
    "full_source_sha256": "a184e3b73febf1176f891ae3e8ae8788e01cac014b7bebc576ffff5d77b45240",
    "source_bytes": 77,
    "full_source_bytes": 140,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0507"
    ],
    "stock_check_error_codes": [
      "E0507"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0507",
        "message": "cannot move out of `*self` which is behind a shared reference"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0507",
        "message": "cannot move out of `*self` which is behind a shared reference"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N22",
    "name": "canonical_copy_with_drop_is_invalid_before_unsupported",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "Copy plus canonical Drop invalid before boundary",
    "status": "invalid",
    "source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}}impl Copy for S{}impl Drop for S{fn drop(&mut self){}}fn run()->i32{0}",
    "full_source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}}impl Copy for S{}impl Drop for S{fn drop(&mut self){}}fn run()->i32{0}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "28105d1725a6aa878e2bc3666fd68856c5b8a76fc136c36553c874cf0f1ef355",
    "full_source_sha256": "40c8296a2090b353985caf943faee4af5bf717ce531501f8075ebbdb6fbb15e1",
    "source_bytes": 137,
    "full_source_bytes": 200,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0184"
    ],
    "stock_check_error_codes": [
      "E0184"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0184",
        "message": "the trait `Copy` cannot be implemented for this type; the type has a destructor"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0184",
        "message": "the trait `Copy` cannot be implemented for this type; the type has a destructor"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N23",
    "name": "foreign_canonical_copy_or_clone_impl_is_orphan_invalid",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "foreign builtin implementation coherence",
    "status": "invalid",
    "source": "impl core::clone::Clone for i32{fn clone(&self)->Self{*self}}fn run()->i32{0}",
    "full_source": "impl core::clone::Clone for i32{fn clone(&self)->Self{*self}}fn run()->i32{0}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "7cfa819e670da64a78167196366837b4da372ca08303da4fcd780277dc2b808e",
    "full_source_sha256": "e9054cfffc9523d7cb2d424544635be014953acd2a8447d93516c8b36201b88a",
    "source_bytes": 77,
    "full_source_bytes": 140,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0117"
    ],
    "stock_check_error_codes": [
      "E0117"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0117",
        "message": "only traits defined in the current crate can be implemented for primitive types"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0117",
        "message": "only traits defined in the current crate can be implemented for primitive types"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N24",
    "name": "explicit_canonical_sized_impl_is_invalid",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "canonical Sized cannot have explicit source impl",
    "status": "invalid",
    "source": "struct S{v:i32}impl core::marker::Sized for S{}fn run()->i32{0}",
    "full_source": "struct S{v:i32}impl core::marker::Sized for S{}fn run()->i32{0}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "bc21b6b4909f3307d01c90ae50b871f3c78fa632bf4daa2e1e6794034ed12c89",
    "full_source_sha256": "a188c6a2790e626a6d16218cee56f60c574ed37c27177b0959ab7432f138b1c3",
    "source_bytes": 63,
    "full_source_bytes": 126,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0322"
    ],
    "stock_check_error_codes": [
      "E0322"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0322",
        "message": "explicit impls for the `Sized` trait are not permitted"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0322",
        "message": "explicit impls for the `Sized` trait are not permitted"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N25",
    "name": "mutable_reference_does_not_gain_copy_permission",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "mutable carrier remains nonCopy",
    "status": "invalid",
    "source": "fn run()->i32{let mut v=1;let r=&mut v;let a=r;let b=r;*a+*b}",
    "full_source": "fn run()->i32{let mut v=1;let r=&mut v;let a=r;let b=r;*a+*b}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "0c7fe47c6a129cfbbf9e3449e48db8ef0aa2c9b5b0144b5614df53d5cb04e655",
    "full_source_sha256": "898d3b104da08d072433dc82fbc471a47ba95c1039fbd41ff5cd98161e093aa6",
    "source_bytes": 61,
    "full_source_bytes": 124,
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
        "message": "use of moved value: `r`"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0382",
        "message": "use of moved value: `r`"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N26",
    "name": "shared_reference_clone_result_escapes_original_referent",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "shared clone result original referent death",
    "status": "invalid",
    "source": "fn run()->i32{let out;{let v=1;let r=&v;out=<&i32 as core::clone::Clone>::clone(&r);}*out}",
    "full_source": "fn run()->i32{let out;{let v=1;let r=&v;out=<&i32 as core::clone::Clone>::clone(&r);}*out}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "74f5dafc034d219be57d291c6398e417ad1e9e611a1d366cd797c4ad0ceba248",
    "full_source_sha256": "6b9dbb6b62ca9f15c158aea28502c32a5993ee06743bc813e631a1ee37e3c0f0",
    "source_bytes": 90,
    "full_source_bytes": 153,
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
        "message": "`v` does not live long enough"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0597",
        "message": "`v` does not live long enough"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N27",
    "name": "wrong_T_U_clone_result_region_despite_concrete_coincidence",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "wrong result region despite matching concrete T U",
    "status": "invalid",
    "source": "fn wrong<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a U{let ignored=x;<&U as core::clone::Clone>::clone(&y)}fn run()->i32{let long=1;let out;{let short=2;out=wrong::<i32,i32>(&long,&short);}*out}",
    "full_source": "fn wrong<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a U{let ignored=x;<&U as core::clone::Clone>::clone(&y)}fn run()->i32{let long=1;let out;{let short=2;out=wrong::<i32,i32>(&long,&short);}*out}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "0eaa1a42ef48fb11b825dc2c2ddd31213cfd0c87c2940f95356ccee287aac52a",
    "full_source_sha256": "90e53a0bd15e31459d616e66511985984f672a544f639ee8d2bbd9ec15731c53",
    "source_bytes": 183,
    "full_source_bytes": 246,
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
    "id": "N28",
    "name": "zero_length_array_field_still_requires_element_copy",
    "group": "invalid",
    "proposed_group": "invalid",
    "category": "array zero element Copy still required",
    "status": "invalid",
    "source": "struct S<T>{v:[T;0]}impl<T:Clone> Clone for S<T>{fn clone(&self)->Self{S{v:self.v.clone()}}}impl<T:Clone> Copy for S<T>{}fn run()->i32{0}",
    "full_source": "struct S<T>{v:[T;0]}impl<T:Clone> Clone for S<T>{fn clone(&self)->Self{S{v:self.v.clone()}}}impl<T:Clone> Copy for S<T>{}fn run()->i32{0}\nfn main(){if run()==0{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 0,
    "expected_stdout": "",
    "classification_observed": true,
    "source_sha256": "bf412fe1134898af32e228772cc2cc1bd137f0450270c80a216570c38dbf2cbd",
    "full_source_sha256": "af2af58e69d4db20ab8acc6a0387ac06e16700027098c2eeaab7f53ea9f334d5",
    "source_bytes": 137,
    "full_source_bytes": 200,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0204"
    ],
    "stock_check_error_codes": [
      "E0204"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0204",
        "message": "the trait `Copy` cannot be implemented for this type"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0204",
        "message": "the trait `Copy` cannot be implemented for this type"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "X29",
    "name": "ordinary_reference_generic_actual_clone_wrapper",
    "group": "legal-excluded",
    "proposed_group": "legal-excluded",
    "category": "ordinary generic actual fence",
    "status": "valid",
    "source": "fn wrap<T:Clone>(x:T)->T{x.clone()}fn run()->i32{let v=20;let r=wrap::<&i32>(&v);*r}",
    "full_source": "fn wrap<T:Clone>(x:T)->T{x.clone()}fn run()->i32{let v=20;let r=wrap::<&i32>(&v);*r}\nfn main(){if run()==20{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 20,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "118565f4ced1cbbf881122ba2800c068ed02f413548f0c7f6bf3872c98fe460d",
    "full_source_sha256": "98bf96d82aec545126789a5d5166710a45a1cf28bcae9cce36d82e6b490546f6",
    "source_bytes": 84,
    "full_source_bytes": 148,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "X30",
    "name": "unavailable_canonical_clone_from_default_demand",
    "group": "legal-excluded",
    "proposed_group": "legal-excluded",
    "category": "unavailable provided default requested",
    "status": "valid",
    "source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}}fn run()->i32{let mut destination=S{v:1};let source=S{v:2};destination.clone_from(&source);destination.v}",
    "full_source": "struct S{v:i32}impl Clone for S{fn clone(&self)->Self{S{v:self.v}}}fn run()->i32{let mut destination=S{v:1};let source=S{v:2};destination.clone_from(&source);destination.v}\nfn main(){if run()==2{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 2,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "ddbd708f697cae3e57af0dbce6312486730dcfd6375910c81bcd4d9af1bf65ae",
    "full_source_sha256": "ddceb3d0dd1eec95b3e59fe27a30f6b8789660c9c47dd9cb5420718b8841ed18",
    "source_bytes": 172,
    "full_source_bytes": 235,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "X31",
    "name": "forced_clone_on_double_shared_reference_requires_depth3_receiver",
    "group": "legal-excluded",
    "proposed_group": "legal-excluded",
    "category": "logical membership distinct from executable depth",
    "status": "valid",
    "source": "fn run()->i32{let v=22;let r=&v;let rr=&r;let cloned=<&&i32 as core::clone::Clone>::clone(&rr);**cloned}",
    "full_source": "fn run()->i32{let v=22;let r=&v;let rr=&r;let cloned=<&&i32 as core::clone::Clone>::clone(&rr);**cloned}\nfn main(){if run()==22{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 22,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "68e4bce9ad409caf4dd2064e5dbdc2be62208d59394ddc1f7c4403ca6aaa34a8",
    "full_source_sha256": "21930f5287200ee4aba950d51a8c0b1c7693cb8b692bbef3df423bcdf284464f",
    "source_bytes": 104,
    "full_source_bytes": 168,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "X32",
    "name": "legal_canonical_drop_only_requires_destructor_execution",
    "group": "legal-excluded",
    "proposed_group": "legal-excluded",
    "category": "legal destructor cannot silently be omitted",
    "status": "valid",
    "source": "struct S{v:i32}impl Drop for S{fn drop(&mut self){self.v=0;}}fn run()->i32{let s=S{v:23};let result=s.v;result}",
    "full_source": "struct S{v:i32}impl Drop for S{fn drop(&mut self){self.v=0;}}fn run()->i32{let s=S{v:23};let result=s.v;result}\nfn main(){if run()==23{println!(\"canonical copy oracle ok\");}}\n",
    "expected": 23,
    "expected_stdout": "canonical copy oracle ok\n",
    "classification_observed": true,
    "source_sha256": "28fc4d7000d2464e5f7f11cfb3467592099b3de0b6e8da40432963667b7e5de6",
    "full_source_sha256": "77d24a043ea0f6d3e99e19ac05d82b229b4c56985791c70a57ed246d0ba5c453",
    "source_bytes": 111,
    "full_source_bytes": 175,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "canonical copy oracle ok\n",
    "stock_runtime_stderr": ""
  }
]
'''
FIXTURES = json.loads(FROZEN_CORPUS_JSON)

def corpus():
    if hashlib.sha256(FROZEN_CORPUS_JSON.encode()).hexdigest() != ACCEPTANCE_CORPUS_SHA256:
        raise ValueError("frozen canonical Copy dictionary bytes changed")
    kinds = {"positive": "valid", "invalid": "invalid", "legal-excluded": "boundary"}
    if len(FIXTURES) != 32 or len({f["name"] for f in FIXTURES}) != 32:
        raise ValueError("expected exactly32 unique canonical Copy fixtures")
    if [sum(f["group"] == g for f in FIXTURES) for g in kinds] != [16, 12, 4]:
        raise ValueError("frozen canonical Copy classifications changed")
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
    result = process([*argv, "-o", output], directory, compile_timeout)
    check = process([*argv, "--emit=metadata", "-o", metadata], directory, compile_timeout)
    fixture = FIXTURES[index]
    build_errors, build_malformed = error_diagnostics(result)
    check_errors, check_malformed = error_diagnostics(check)
    row = {"record": "stock_case", "index": index, "name": case.name,
           "kind": case.kind, "source": case.source,
           "source_sha256": hashlib.sha256(case.source.encode()).hexdigest(),
           "stock_compile": result, "stock_check": check,
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
        if fixture["id"] == "N27":
            required = [{"level": "error", "code": None,
                         "message": "lifetime may not live long enough"}]
            if build_errors != required or check_errors != required:
                row["failures"].append("N27 requires exact structured code-less lifetime diagnostic")
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
            prefix="jr-canonical-copy-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "canonical-copy-v1",
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
                raise SystemExit(f"canonical Copy discrepancy, frozen sources not changed: {args.output}")
        record({"record": "summary", **totals, "passed": totals["failure"] == 0})


if __name__ == "__main__":
    main()
