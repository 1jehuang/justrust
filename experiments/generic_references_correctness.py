"""Frozen exact32 generic references and borrowed receiver differential corpus.

Sixteen valid, twelve invalid and four legal excluded sources. Uses bounded shared
Case/run_case/process, supplied native binary workers1/8 CHECK/BUILD/runtime.
N18 requires the exact stock structured code-less lifetime error. No native build.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "5d13d72b1a7b006e507621c6a43a339ea92344b185655ca0c31113f81d7a38c7"

# Authoritative frozen stock oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "fn id<T>(x:&T)->&T{x}fn forward<T>(x:&T)->&T{id::<T>(x)}fn run()->i32{let x=7;*forward::<i32>(&x)}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "generic_shared_identity_and_rigid_forwarding",
    "proposed_group": "positive",
    "source": "fn id<T>(x:&T)->&T{x}fn forward<T>(x:&T)->&T{id::<T>(x)}fn run()->i32{let x=7;*forward::<i32>(&x)}",
    "source_bytes": 165,
    "source_sha256": "2c3f2acf0e17c51e79a99abe87b743f97bb3814cc352bfa2531063818979125a",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "fn id<T>(x:&mut T)->&mut T{x}fn run()->i32{let mut x=3;let r=id::<i32>(&mut x);*r=7;x}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "generic_mutable_identity_and_write",
    "proposed_group": "positive",
    "source": "fn id<T>(x:&mut T)->&mut T{x}fn run()->i32{let mut x=3;let r=id::<i32>(&mut x);*r=7;x}",
    "source_bytes": 153,
    "source_sha256": "726a7c7f04193d7282f5a1331e39f180a77ece8858e66011c5b5e17ea74250b8",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "struct S{x:i32}fn twice<T>(r:&T)->&T{let a=r;let b=r;let _=a;b}fn run()->i32{let s=S{x:7};let r=twice::<S>(&s);r.x}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "shared_carrier_copy_noncopy_referent",
    "proposed_group": "positive",
    "source": "struct S{x:i32}fn twice<T>(r:&T)->&T{let a=r;let b=r;let _=a;b}fn run()->i32{let s=S{x:7};let r=twice::<S>(&s);r.x}",
    "source_bytes": 182,
    "source_sha256": "4cb58b0be1c669e76c2651482702288eb126cdfc7c575e11deebdeb6d95ab93f",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "struct S{x:i32,y:i32}fn id<T>(x:&mut T)->&mut T{x}fn run()->i32{let mut s=S{x:1,y:2};let r=id::<i32>(&mut s.x);s.y=3;*r=4;let mut a=[1,2];*id::<i32>(&mut a[0])=4;let mut p=(1,2);*id::<i32>(&mut p.0)=4;s.x+s.y+a[0]+p.0-8}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "reference_free_product_referents_and_disjoint_fields",
    "proposed_group": "positive",
    "source": "struct S{x:i32,y:i32}fn id<T>(x:&mut T)->&mut T{x}fn run()->i32{let mut s=S{x:1,y:2};let r=id::<i32>(&mut s.x);s.y=3;*r=4;let mut a=[1,2];*id::<i32>(&mut a[0])=4;let mut p=(1,2);*id::<i32>(&mut p.0)=4;s.x+s.y+a[0]+p.0-8}",
    "source_bytes": 287,
    "source_sha256": "55d14ad2f8aaecd0739a25c823cfc21f91d9dcf053d569537694a490f93956dc",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "fn first<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a T{let _=y;x}fn run()->i32{let x=7;let r;{let y=3;r=first::<i32,i32>(&x,&y);}*r}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "T_U_coincidence_preserves_declared_region_authority",
    "proposed_group": "positive",
    "source": "fn first<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a T{let _=y;x}fn run()->i32{let x=7;let r;{let y=3;r=first::<i32,i32>(&x,&y);}*r}",
    "source_bytes": 188,
    "source_sha256": "015b002ea5cfbf576b2e2933a00d3debcec17e17980de062d2d62318e6f8333a",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn run()->i32{let s=S{x:3};let a=s.get();let r=&s;a+r.get()+1}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "inherent_shared_receiver_preserves_owner",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn run()->i32{let s=S{x:3};let a=s.get();let r=&s;a+r.get()+1}",
    "source_bytes": 178,
    "source_sha256": "abc76d2bfdb338967483de23a45bd501c06ba79153e313f92f138cc33bcd13bb",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn set(&mut self,x:i32){self.x=x;}}fn run()->i32{let mut s=S{x:0};s.set(7);s.x}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "inherent_mut_receiver_write",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn set(&mut self,x:i32){self.x=x;}}fn run()->i32{let mut s=S{x:0};s.set(7);s.x}",
    "source_bytes": 168,
    "source_sha256": "c84ff04c4728de70c6e413ff0e101df562b567d3aa19f2c9508c374294da88b5",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn get(&self,other:&i32)->&i32{let _=other;&self.x}}fn run()->i32{let s=S{x:7};let r;{let other=3;r=s.get(&other);}*r}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "receiver_output_elision_over_other_reference_input",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn get(&self,other:&i32)->&i32{let _=other;&self.x}}fn run()->i32{let s=S{x:7};let r;{let other=3;r=s.get(&other);}*r}",
    "source_bytes": 207,
    "source_sha256": "be2bd2dc8f8838dafdeaf934c99c711f8a7461792985bf0e487eb6e8bb79dd4e",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "trait Get{fn get<'a>(&'a self)->&'a i32;}struct S{x:i32}impl Get for S{fn get<'z>(&'z self)->&'z i32{&self.x}}fn run()->i32{let s=S{x:7};*<S as Get>::get(&s)}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "named_receiver_alpha_normalized_contract",
    "proposed_group": "positive",
    "source": "trait Get{fn get<'a>(&'a self)->&'a i32;}struct S{x:i32}impl Get for S{fn get<'z>(&'z self)->&'z i32{&self.x}}fn run()->i32{let s=S{x:7};*<S as Get>::get(&s)}",
    "source_bytes": 225,
    "source_sha256": "749ab186b542a037e3078686bf08f94b0b04d690c37446f7873db246969bd6d5",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "trait Field{fn field(&self)->&i32;fn field_mut(&mut self)->&mut i32;}struct S{x:i32}impl Field for S{fn field(&self)->&i32{&self.x}fn field_mut(&mut self)->&mut i32{&mut self.x}}fn shared<T:Field>(x:&T)->&i32{x.field()}fn mutable<T:Field>(x:&mut T)->&mut i32{x.field_mut()}fn run()->i32{let mut s=S{x:3};*mutable::<S>(&mut s)=7;*shared::<S>(&s)}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "trait_projected_shared_mutable_returns_and_generic_forwarding",
    "proposed_group": "positive",
    "source": "trait Field{fn field(&self)->&i32;fn field_mut(&mut self)->&mut i32;}struct S{x:i32}impl Field for S{fn field(&self)->&i32{&self.x}fn field_mut(&mut self)->&mut i32{&mut self.x}}fn shared<T:Field>(x:&T)->&i32{x.field()}fn mutable<T:Field>(x:&mut T)->&mut i32{x.field_mut()}fn run()->i32{let mut s=S{x:3};*mutable::<S>(&mut s)=7;*shared::<S>(&s)}",
    "source_bytes": 412,
    "source_sha256": "9b5798607c1060eb0bfe0ff1e1a8df81d195ddf7e199c7b799ecb2beab4ccaad",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "trait Value{fn value(&self)->i32;}trait Take{fn take(x:&Self)->&i32;}struct S{x:i32}impl Value for S{fn value(&self)->i32{self.x}}impl Take for S{fn take(x:&Self)->&i32{&x.x}}struct Wrap<T>{x:T}impl<T:Value> Value for Wrap<T>{fn value(&self)->i32{self.x.value()}}fn read<T:Take>(x:&T)->&i32{T::take(x)}fn run()->i32{let s=S{x:3};let a=*read::<S>(&s);let w=Wrap::<S>{x:S{x:4}};a+w.value()}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "conditional_trait_dispatch_and_static_trait_result_opacity",
    "proposed_group": "positive",
    "source": "trait Value{fn value(&self)->i32;}trait Take{fn take(x:&Self)->&i32;}struct S{x:i32}impl Value for S{fn value(&self)->i32{self.x}}impl Take for S{fn take(x:&Self)->&i32{&x.x}}struct Wrap<T>{x:T}impl<T:Value> Value for Wrap<T>{fn value(&self)->i32{self.x.value()}}fn read<T:Take>(x:&T)->&i32{T::take(x)}fn run()->i32{let s=S{x:3};let a=*read::<S>(&s);let w=Wrap::<S>{x:S{x:4}};a+w.value()}",
    "source_bytes": 455,
    "source_sha256": "960d589febe3049f3aeab6f9f41c0a152347d3d5b1958385922dac98c59a5a6b",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn pick<'a>(s:&'a S,count:&mut i32)->&'a S{*count+=1;s}fn run()->i32{let s=S{x:6};let mut count=0;let x=pick(&s,&mut count).get();x+count}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "genuine_reference_call_receiver_once",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn pick<'a>(s:&'a S,count:&mut i32)->&'a S{*count+=1;s}fn run()->i32{let s=S{x:6};let mut count=0;let x=pick(&s,&mut count).get();x+count}",
    "source_bytes": 254,
    "source_sha256": "acfa7e19d11d2c2fd2e00a582f82b825184b0dc707e365b574c5fe59643e1b3f",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn set(&mut self,a:i32,b:i32){self.x=a+b;}}fn pick<'a>(s:&'a mut S,c:&mut i32)->&'a mut S{*c=*c*10+1;s}fn arg(c:&mut i32,x:i32)->i32{*c=*c*10+x;x}fn run()->i32{let mut s=S{x:0};let mut c=0;pick(&mut s,&mut c).set(arg(&mut c,2),arg(&mut c,3));if c==123{s.x+2}else{0}}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "receiver_and_explicit_arguments_once_ltr",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn set(&mut self,a:i32,b:i32){self.x=a+b;}}fn pick<'a>(s:&'a mut S,c:&mut i32)->&'a mut S{*c=*c*10+1;s}fn arg(c:&mut i32,x:i32)->i32{*c=*c*10+x;x}fn run()->i32{let mut s=S{x:0};let mut c=0;pick(&mut s,&mut c).set(arg(&mut c,2),arg(&mut c,3));if c==123{s.x+2}else{0}}",
    "source_bytes": 355,
    "source_sha256": "fd1148b77719334b3ca5ddd6b6b23c6c2a1d71b559e4bb0b32aaec1fdc04d754",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,x:i32){self.x=x;}}fn run()->i32{let mut s=S{x:6};s.set(s.get()+1);s.x}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "implicit_mut_receiver_shared_read_then_activation",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,x:i32){self.x=x;}}fn run()->i32{let mut s=S{x:6};s.set(s.get()+1);s.x}",
    "source_bytes": 202,
    "source_sha256": "e93b64aeb040c4ce0a6f10dea81e2fee75c6fe3444881ac861b5c79bd5f13acc",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "fn set(r:&mut i32,x:i32){*r=x;}fn run()->i32{let mut x=6;let r=&mut x;set(r,*r+1);x}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "implicit_mut_argument_reborrow_shared_read",
    "proposed_group": "positive",
    "source": "fn set(r:&mut i32,x:i32){*r=x;}fn run()->i32{let mut x=6;let r=&mut x;set(r,*r+1);x}",
    "source_bytes": 151,
    "source_sha256": "72bd6fe688e864858e240aa4d0627838441b70b0adb0d7a16d5d1cde89f92cbe",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,x:i32)->&mut i32{self.x=x;&mut self.x}}fn run()->i32{let mut s=S{x:4};let mut i=0;while i<3{let r=s.set(s.get()+1);let _=*r;i+=1;}s.x}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "positive",
    "name": "returned_mut_ref_last_use_and_per_iteration_two_phase_loop",
    "proposed_group": "positive",
    "source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,x:i32)->&mut i32{self.x=x;&mut self.x}}fn run()->i32{let mut s=S{x:4};let mut i=0;while i<3{let r=s.set(s.get()+1);let _=*r;i+=1;}s.x}",
    "source_bytes": 266,
    "source_sha256": "483c8463b25e901fcee66ef9b8a7b263be7a13441ccd2f82d209eb6a505750e5",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": "E0507",
    "expected_stdout": "",
    "full_source": "fn bad<T>(x:&T)->T{*x}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unused_generic_body_move_through_deref",
    "proposed_group": "negative",
    "source": "fn bad<T>(x:&T)->T{*x}fn run()->i32{7}",
    "source_bytes": 57,
    "source_sha256": "00035202a566b5d5dd504e64ca752ea1df9bcad77020f6a99b2dacf763da3d5f",
    "stock_check_error_codes": [
      "E0507"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0507"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "lifetime may not live long enough",
    "expected_stdout": "",
    "full_source": "fn wrong<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a U{let _=x;y}fn run()->i32{let x=7;let y=3;*wrong::<i32,i32>(&x,&y)}\nfn main(){run();}\n",
    "group": "negative",
    "name": "wrong_result_region_despite_T_U_concrete_coincidence",
    "proposed_group": "negative",
    "source": "fn wrong<'a,'b,T,U>(x:&'a T,y:&'b U)->&'a U{let _=x;y}fn run()->i32{let x=7;let y=3;*wrong::<i32,i32>(&x,&y)}",
    "source_bytes": 128,
    "source_sha256": "28bc5f76170439ae6dde88daf5934c79ce87b338bc118b49bce22331a595b24c",
    "stock_check_error_codes": [],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [],
    "stock_runtime_status": null,
    "stock_error_contains": "lifetime may not live long enough"
  },
  {
    "classification_observed": true,
    "expected": "E0506",
    "expected_stdout": "",
    "full_source": "fn id<T>(x:&mut T)->&mut T{x}fn run()->i32{let mut x=1;let r=&mut x;let child=id::<i32>(r);*r=3;*child}\nfn main(){run();}\n",
    "group": "negative",
    "name": "mutable_reborrow_parent_conflict_live_child",
    "proposed_group": "negative",
    "source": "fn id<T>(x:&mut T)->&mut T{x}fn run()->i32{let mut x=1;let r=&mut x;let child=id::<i32>(r);*r=3;*child}",
    "source_bytes": 122,
    "source_sha256": "6e3a63c33c0a5965061bf1f2a0824ac63f86050cc75280589c8ff76e1e275365",
    "stock_check_error_codes": [
      "E0506"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0506"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0053",
    "expected_stdout": "",
    "full_source": "trait Get{fn get(&self)->i32;}struct S{x:i32}impl Get for S{fn get(&mut self)->i32{self.x}}fn run()->i32{7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "unused_impl_borrowed_receiver_kind_mismatch",
    "proposed_group": "negative",
    "source": "trait Get{fn get(&self)->i32;}struct S{x:i32}impl Get for S{fn get(&mut self)->i32{self.x}}fn run()->i32{7}",
    "source_bytes": 126,
    "source_sha256": "bd81864ef385653cfca7c7ad6b70f1c553b15f31829b29d52e01463e3a83c867",
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
    "expected": "E0502",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,x:i32){self.x=x;}}fn run()->i32{let mut s=S{x:7};S::set(&mut s,s.get());7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "explicit_mut_receiver_immediate_conflict",
    "proposed_group": "negative",
    "source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}fn set(&mut self,x:i32){self.x=x;}}fn run()->i32{let mut s=S{x:7};S::set(&mut s,s.get());7}",
    "source_bytes": 158,
    "source_sha256": "82aad341d6ed1b6ac94c934754c6201908b48ab7a69376dff8e4cadfe5ae8747",
    "stock_check_error_codes": [
      "E0502"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0502"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0503",
    "expected_stdout": "",
    "full_source": "fn set(r:&mut i32,x:i32){*r=x;}fn run()->i32{let mut x=7;let r=&mut x;set(&mut *r,*r);7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "explicit_mut_argument_immediate_conflict",
    "proposed_group": "negative",
    "source": "fn set(r:&mut i32,x:i32){*r=x;}fn run()->i32{let mut x=7;let r=&mut x;set(&mut *r,*r);7}",
    "source_bytes": 107,
    "source_sha256": "db8e50ccf421026c9f1cf8b5d0a953ec28538cec3b334c74318d1afc22de172a",
    "stock_check_error_codes": [
      "E0503"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0503"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0506",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}impl S{fn set(&mut self,x:i32){self.x=x;}}fn run()->i32{let mut s=S{x:7};s.set({s.x=3;7});7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "reservation_blocks_argument_write",
    "proposed_group": "negative",
    "source": "struct S{x:i32}impl S{fn set(&mut self,x:i32){self.x=x;}}fn run()->i32{let mut s=S{x:7};s.set({s.x=3;7});7}",
    "source_bytes": 126,
    "source_sha256": "b198e63f10c1e76516b8d5f67fd42665e1b9319d7531131cee049637fc7ad9f4",
    "stock_check_error_codes": [
      "E0506"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0506"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0505",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}impl S{fn set(&mut self,x:i32){self.x=x;}}fn consume(s:S)->i32{s.x}fn run()->i32{let mut s=S{x:7};s.set(consume(s));7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "reservation_blocks_argument_move",
    "proposed_group": "negative",
    "source": "struct S{x:i32}impl S{fn set(&mut self,x:i32){self.x=x;}}fn consume(s:S)->i32{s.x}fn run()->i32{let mut s=S{x:7};s.set(consume(s));7}",
    "source_bytes": 152,
    "source_sha256": "a4a5a3389b106919c6961d9275fbe743c8e3f120c52b8fffffe5ff912ddf7476",
    "stock_check_error_codes": [
      "E0505"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0505"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0499",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}impl S{fn set(&mut self,x:i32)->i32{self.x=x;x}}fn run()->i32{let mut s=S{x:7};s.set(s.set(3));7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "overlapping_second_mutable_reservation",
    "proposed_group": "negative",
    "source": "struct S{x:i32}impl S{fn set(&mut self,x:i32)->i32{self.x=x;x}}fn run()->i32{let mut s=S{x:7};s.set(s.set(3));7}",
    "source_bytes": 131,
    "source_sha256": "903d8b5f6895b38ace92e8ef1eff9feb9742979bd0bea4a7ecb7628ec6d6e191",
    "stock_check_error_codes": [
      "E0499"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0499"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0502",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}impl S{fn set(&mut self,x:&i32){self.x=*x;}}fn run()->i32{let mut s=S{x:7};s.set(&s.x);7}\nfn main(){run();}\n",
    "group": "negative",
    "name": "shared_argument_loan_live_at_activation",
    "proposed_group": "negative",
    "source": "struct S{x:i32}impl S{fn set(&mut self,x:&i32){self.x=*x;}}fn run()->i32{let mut s=S{x:7};s.set(&s.x);7}",
    "source_bytes": 123,
    "source_sha256": "7fbfb4e11da8e3e8d220c7829abdd03ea5bd914923d1250311914d66eb5689d4",
    "stock_check_error_codes": [
      "E0502"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0502"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0506",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}impl S{fn field(&mut self)->&mut i32{&mut self.x}}fn run()->i32{let mut s=S{x:7};let r=s.field();s.x=3;*r}\nfn main(){run();}\n",
    "group": "negative",
    "name": "returned_ref_keeps_activated_loan_live",
    "proposed_group": "negative",
    "source": "struct S{x:i32}impl S{fn field(&mut self)->&mut i32{&mut self.x}}fn run()->i32{let mut s=S{x:7};let r=s.field();s.x=3;*r}",
    "source_bytes": 140,
    "source_sha256": "3843d6760ad430f9cce640190a2374c40dd8b45b9a969a1c4f275ca2096a5bdd",
    "stock_check_error_codes": [
      "E0506"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0506"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": "E0597",
    "expected_stdout": "",
    "full_source": "struct S{x:i32}impl S{fn get(&self)->&i32{&self.x}}fn run()->i32{let r;{let s=S{x:7};r=s.get();}*r}\nfn main(){run();}\n",
    "group": "negative",
    "name": "borrowed_result_owner_storage_dead",
    "proposed_group": "negative",
    "source": "struct S{x:i32}impl S{fn get(&self)->&i32{&self.x}}fn run()->i32{let r;{let s=S{x:7};r=s.get();}*r}",
    "source_bytes": 118,
    "source_sha256": "9c38ca27d6ee3868516b81448269d38ead8bbf054020501d0d38b11d2bee1f1b",
    "stock_check_error_codes": [
      "E0597"
    ],
    "stock_check_status": 1,
    "stock_compile_status": 1,
    "stock_error_codes": [
      "E0597"
    ],
    "stock_runtime_status": null
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "struct Wrap<T>{x:T}fn id<T>(x:T)->T{x}fn run()->i32{let x=7;let r=id::<&i32>(&x);let w=Wrap::<&i32>{x:r};*w.x}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "reference_type_actual_and_hidden_reference_nominal",
    "proposed_group": "legalunsupported",
    "source": "struct Wrap<T>{x:T}fn id<T>(x:T)->T{x}fn run()->i32{let x=7;let r=id::<&i32>(&x);let w=Wrap::<&i32>{x:r};*w.x}",
    "source_bytes": 177,
    "source_sha256": "de8bbc2c5fab8920caaa0037a98040f269ff7bfb188e24492bdafca25319df18",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "fn run()->i32{let x=7;let r=&x;let nested=&&x;let p=(r,nested);*p.0+**p.1-7}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "nested_reference_or_reference_aggregate",
    "proposed_group": "legalunsupported",
    "source": "fn run()->i32{let x=7;let r=&x;let nested=&&x;let p=(r,nested);*p.0+**p.1-7}",
    "source_bytes": 143,
    "source_sha256": "ed5fc6e077220ebd8b87197522c235182931fab23d1d2c8f4d4abdde845d6be1",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn run()->i32{S{x:7}.get()}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "owned_rvalue_borrowed_receiver_temporary",
    "proposed_group": "legalunsupported",
    "source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn run()->i32{S{x:7}.get()}",
    "source_bytes": 143,
    "source_sha256": "7f47128a4e2812b8a062a9b15b78dc17dfd8cfc9aed3e12617169e1907c20df5",
    "stock_check_error_codes": [],
    "stock_check_status": 0,
    "stock_compile_status": 0,
    "stock_error_codes": [],
    "stock_runtime_status": 0
  },
  {
    "classification_observed": true,
    "expected": 7,
    "expected_stdout": "generic references oracle ok\n",
    "full_source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn run()->i32{let a=[S{x:7}];let i=0usize;let r=&a[i];let rr=&r;rr.get()}\nfn main(){if run()==7{println!(\"generic references oracle ok\");}}\n",
    "group": "legalunsupported",
    "name": "dynamic_index_borrow_or_autoderef_chain",
    "proposed_group": "legalunsupported",
    "source": "struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn run()->i32{let a=[S{x:7}];let i=0usize;let r=&a[i];let rr=&r;rr.get()}",
    "source_bytes": 189,
    "source_sha256": "cfd4d6b3fec8006ec962ee9a8ef6a199365f1287a7d75df5d1d04d28abe3a6b5",
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
        if fixture["name"] == "wrong_result_region_despite_T_U_concrete_coincidence":
            for phase in (result, check):
                diagnostics = []
                for line in phase["stderr"].splitlines():
                    try:
                        diagnostic = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    if diagnostic.get("level") == "error" and diagnostic.get("spans"):
                        diagnostics.append(diagnostic)
                if (len(diagnostics) != 1 or diagnostics[0].get("code") is not None
                        or diagnostics[0].get("message") != fixture["stock_error_contains"]):
                    row["failures"].append("N18 requires exact structured code-less lifetime diagnostic")
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


def native_case(case, index, directory, binary, rustc, compile_timeout, run_timeout):
    row = run_case(case, index, directory, binary, rustc, compile_timeout, run_timeout)
    if case.kind == "invalid":
        for worker in row["workers"]:
            for mode in ("compile", "check"):
                text = worker[mode]["stderr"].lower()
                if any(marker in text for marker in ("panic", "parse", "unexpected",
                                                     "unsupported", "not supported")):
                    row["failures"].append("invalid fixture requires semantic rejection, not parser/unsupported failure")
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
    rustc = args.rustc.absolute()  # Preserve a rustup proxy's executable name.
    if not rustc.is_file():
        parser.error("--rustc must name an existing executable")
    binary = args.binary.resolve(strict=True) if args.binary else None
    cases = list(corpus())
    assert len(cases) == 32 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-generic-references-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "generic-references-v1",
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
                row = native_case(case, index, directory / str(index), binary, rustc,
                               args.compile_timeout, args.run_timeout)
            row["expected_stdout"] = case.stdout
            record(row)
            totals[row["outcome"]] += 1
            print(f"{case.name}: {row['outcome']}", flush=True)
        record({"record": "summary", **totals, "passed": totals["failure"] == 0})
        if totals["failure"]:
            raise SystemExit(f"{totals['failure']} generic reference discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
