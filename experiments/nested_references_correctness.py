"""Frozen exact32 bounded two-layer shared-reference differential corpus.

Sixteen positives, twelve invalids and four legal exclusions. Uses the existing
shared bounded runner and a supplied native binary with workers1/8. No native
build is performed. Stock CHECK is metadata-only with exact JSON diagnostics.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case

ACCEPTANCE_CORPUS_SHA256 = "0554a1035124644fb649691423b3706888c4aa6395437f92c493c298a6b49432"

# Original observed dictionary bytes, not reconstructed or source-rewritten.
FROZEN_CORPUS_JSON = r'''[
  {
    "id": "P01",
    "name": "nested_shared_identity_preserves_both_carrier_layers",
    "group": "positive",
    "category": "both position mappings",
    "source": "fn id<'o,'i,T>(x:&'o &'i T)->&'o &'i T{x}\nfn run()->i32{let v=11;let r=&v;let rr=id(&r);let oracle_observation0=**rr;oracle_observation0}",
    "full_source": "fn id<'o,'i,T>(x:&'o &'i T)->&'o &'i T{x}\nfn run()->i32{let v=11;let r=&v;let rr=id(&r);let oracle_observation0=**rr;oracle_observation0}\nfn main(){if run()==11{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "7da374e27c1648ec784ff59a48e619a397f553633025f0773d3b2a7fd24b2506",
    "expected": 11,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "7da374e27c1648ec784ff59a48e619a397f553633025f0773d3b2a7fd24b2506",
    "source_bytes": 204,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P02",
    "name": "load_inner_transports_original_referent_origins",
    "group": "positive",
    "category": "inner load provenance",
    "source": "fn inner<'o,'i,T>(x:&'o &'i T)->&'i T{*x}\nfn run()->i32{let v=12;let r=&v;let x=inner(&r);let oracle_observation0=*x;oracle_observation0}",
    "full_source": "fn inner<'o,'i,T>(x:&'o &'i T)->&'i T{*x}\nfn run()->i32{let v=12;let r=&v;let x=inner(&r);let oracle_observation0=*x;oracle_observation0}\nfn main(){if run()==12{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "a61470a9de1e39615e6a4e9feeed47cf09b48c3668e6ebdca4cdca63226e78bd",
    "expected": 12,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "a61470a9de1e39615e6a4e9feeed47cf09b48c3668e6ebdca4cdca63226e78bd",
    "source_bytes": 204,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P03",
    "name": "loaded_inner_escapes_temporary_outer_handle_scope",
    "group": "positive",
    "category": "outer loan separation",
    "source": "fn inner<'o,'i,T>(x:&'o &'i T)->&'i T{*x}\nfn run()->i32{let v=13;let r=&v;let out;{let rr=&r;out=inner(rr);}let oracle_observation0=*out;oracle_observation0}",
    "full_source": "fn inner<'o,'i,T>(x:&'o &'i T)->&'i T{*x}\nfn run()->i32{let v=13;let r=&v;let out;{let rr=&r;out=inner(rr);}let oracle_observation0=*out;oracle_observation0}\nfn main(){if run()==13{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "53f76a48527c310f5721b3790b7df6c58c81551867f0b97058583ce7a62b309b",
    "expected": 13,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "53f76a48527c310f5721b3790b7df6c58c81551867f0b97058583ce7a62b309b",
    "source_bytes": 224,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P04",
    "name": "loaded_inner_escapes_local_inner_binding_scope",
    "group": "positive",
    "category": "handle storage separation",
    "source": "fn run()->i32{let v=14;let out;{let r=&v;let rr=&r;out=*rr;}let oracle_observation0=*out;oracle_observation0}",
    "full_source": "fn run()->i32{let v=14;let out;{let r=&v;let rr=&r;out=*rr;}let oracle_observation0=*out;oracle_observation0}\nfn main(){if run()==14{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "f7fe1464580a34a6632a2d37dd51a4718c2cc92e80c3e98328857625fe0522f8",
    "expected": 14,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "f7fe1464580a34a6632a2d37dd51a4718c2cc92e80c3e98328857625fe0522f8",
    "source_bytes": 176,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P05",
    "name": "generic_reference_free_T_inner_load_and_forward",
    "group": "positive",
    "category": "rigid generic referent",
    "source": "struct Pair<T>{value:T,tag:i32}\nfn inner<'o,'i,T>(x:&'o &'i T)->&'i T{*x}\nfn forward<'o,'i,T>(x:&'o &'i T)->&'i T{inner(x)}\nfn run()->i32{let v=Pair{value:15,tag:1};let r=&v;let out=forward(&r);let oracle_observation0=out.value+out.tag;oracle_observation0}",
    "full_source": "struct Pair<T>{value:T,tag:i32}\nfn inner<'o,'i,T>(x:&'o &'i T)->&'i T{*x}\nfn forward<'o,'i,T>(x:&'o &'i T)->&'i T{inner(x)}\nfn run()->i32{let v=Pair{value:15,tag:1};let r=&v;let out=forward(&r);let oracle_observation0=out.value+out.tag;oracle_observation0}\nfn main(){if run()==16{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "c27ed25e4188a1ed31dc7009508cafdb9a47847fe3491ee97b24b218aa3c7f70",
    "expected": 16,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "c27ed25e4188a1ed31dc7009508cafdb9a47847fe3491ee97b24b218aa3c7f70",
    "source_bytes": 323,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P06",
    "name": "plain_shared_carrier_copy_with_noncopy_reference_free_T",
    "group": "positive",
    "category": "intrinsic shared carrier copying",
    "source": "struct Token{v:i32}\nfn run()->i32{let v=Token{v:16};let r=&v;let rr=&r;let copied=rr;let a=*rr;let b=*copied;let oracle_observation0=a.v+b.v;oracle_observation0}",
    "full_source": "struct Token{v:i32}\nfn run()->i32{let v=Token{v:16};let r=&v;let rr=&r;let copied=rr;let a=*rr;let b=*copied;let oracle_observation0=a.v+b.v;oracle_observation0}\nfn main(){if run()==32{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "f1da2cbcee5fc4e9557bd008daa2c0d6f5d2ef31996940cd8345ed2a0246d55a",
    "expected": 32,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "f1da2cbcee5fc4e9557bd008daa2c0d6f5d2ef31996940cd8345ed2a0246d55a",
    "source_bytes": 228,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P07",
    "name": "distinct_named_lifetimes_imply_inner_outlives_outer",
    "group": "positive",
    "category": "implied WF",
    "source": "fn shorten<'o,'i,T>(x:&'o &'i T)->&'o T{*x}\nfn run()->i32{let v=17;let r=&v;let out=shorten(&r);let oracle_observation0=*out;oracle_observation0}",
    "full_source": "fn shorten<'o,'i,T>(x:&'o &'i T)->&'o T{*x}\nfn run()->i32{let v=17;let r=&v;let out=shorten(&r);let oracle_observation0=*out;oracle_observation0}\nfn main(){if run()==17{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "ea02f0f4d7a1b29983c8d5815e52b002d847eb7ab4d0f3322e08662d7044805b",
    "expected": 17,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "ea02f0f4d7a1b29983c8d5815e52b002d847eb7ab4d0f3322e08662d7044805b",
    "source_bytes": 212,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P08",
    "name": "explicit_inner_result_lifetime_not_outer_lifetime",
    "group": "positive",
    "category": "named result region",
    "source": "fn inner<'outer,'inner>(x:&'outer &'inner i32)->&'inner i32{*x}\nfn run()->i32{let v=18;let out;{let r=&v;out=inner(&r);}let oracle_observation0=*out;oracle_observation0}",
    "full_source": "fn inner<'outer,'inner>(x:&'outer &'inner i32)->&'inner i32{*x}\nfn run()->i32{let v=18;let out;{let r=&v;out=inner(&r);}let oracle_observation0=*out;oracle_observation0}\nfn main(){if run()==18{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "569eb5a4807d1ab389586ff8bcdd36a7bb65f2d5fe012be0710dcbbaeeae6ecb",
    "expected": 18,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "569eb5a4807d1ab389586ff8bcdd36a7bb65f2d5fe012be0710dcbbaeeae6ecb",
    "source_bytes": 236,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P09",
    "name": "call_returns_inner_with_original_argument_origins",
    "group": "positive",
    "category": "call substitution",
    "source": "fn get<'o,'i>(x:&'o &'i i32)->&'i i32{*x}\nfn run()->i32{let v=19;let r=&v;let result=get(&r);let oracle_observation0=*result;oracle_observation0}",
    "full_source": "fn get<'o,'i>(x:&'o &'i i32)->&'i i32{*x}\nfn run()->i32{let v=19;let r=&v;let result=get(&r);let oracle_observation0=*result;oracle_observation0}\nfn main(){if run()==19{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "9369f2a39ff31a5dc613efafb0b1135b3b89c00fc7425b5dd9b77f859f2a1c10",
    "expected": 19,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "9369f2a39ff31a5dc613efafb0b1135b3b89c00fc7425b5dd9b77f859f2a1c10",
    "source_bytes": 212,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P10",
    "name": "genuine_nested_call_once_forwarding_preserves_layer_positions",
    "group": "positive",
    "category": "once-only call composition",
    "source": "fn tick<'o,'i>(x:&'o &'i i32,n:&mut i32)->&'o &'i i32{*n+=1;x}\nfn inner<'o,'i>(x:&'o &'i i32)->&'i i32{*x}\nfn run()->i32{let v=20;let r=&v;let mut n=0;let out=inner(tick(&r,&mut n));let oracle_observation0=*out;let oracle_observation1=n;oracle_observation0*100+oracle_observation1}",
    "full_source": "fn tick<'o,'i>(x:&'o &'i i32,n:&mut i32)->&'o &'i i32{*n+=1;x}\nfn inner<'o,'i>(x:&'o &'i i32)->&'i i32{*x}\nfn run()->i32{let v=20;let r=&v;let mut n=0;let out=inner(tick(&r,&mut n));let oracle_observation0=*out;let oracle_observation1=n;oracle_observation0*100+oracle_observation1}\nfn main(){if run()==2001{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "db30a19104db49d6155ab0ce3bc8c9949b081947f321dab754482a2c0beeceaa",
    "expected": 2001,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "db30a19104db49d6155ab0ce3bc8c9949b081947f321dab754482a2c0beeceaa",
    "source_bytes": 350,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P11",
    "name": "joined_inner_load_unions_original_referent_origins",
    "group": "positive",
    "category": "join",
    "source": "fn choose<'o,'i>(a:&'o &'i i32,b:&'o &'i i32,flag:bool)->&'i i32{let rr;if flag{rr=a;}else{rr=b;}*rr}\nfn run()->i32{let a=21;let b=22;let ra=&a;let rb=&b;let oracle_observation0=*choose(&ra,&rb,true);let oracle_observation1=*choose(&ra,&rb,false);oracle_observation0*100+oracle_observation1}",
    "full_source": "fn choose<'o,'i>(a:&'o &'i i32,b:&'o &'i i32,flag:bool)->&'i i32{let rr;if flag{rr=a;}else{rr=b;}*rr}\nfn run()->i32{let a=21;let b=22;let ra=&a;let rb=&b;let oracle_observation0=*choose(&ra,&rb,true);let oracle_observation1=*choose(&ra,&rb,false);oracle_observation0*100+oracle_observation1}\nfn main(){if run()==2122{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "842b053c4106c5f85e4843398bd9c714e32406ce5e772c0d2ad4db08cae49f22",
    "expected": 2122,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "842b053c4106c5f85e4843398bd9c714e32406ce5e772c0d2ad4db08cae49f22",
    "source_bytes": 360,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P12",
    "name": "rebind_nested_carrier_then_load_uses_current_origins",
    "group": "positive",
    "category": "carrier assignment",
    "source": "fn run()->i32{let a=23;let b=24;let ra=&a;let rb=&b;let mut rr=&ra;let first=*rr;rr=&rb;let second=*rr;let oracle_observation0=*first;let oracle_observation1=*second;oracle_observation0*100+oracle_observation1}",
    "full_source": "fn run()->i32{let a=23;let b=24;let ra=&a;let rb=&b;let mut rr=&ra;let first=*rr;rr=&rb;let second=*rr;let oracle_observation0=*first;let oracle_observation1=*second;oracle_observation0*100+oracle_observation1}\nfn main(){if run()==2324{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "bbdbdf38a2b03e8c79419fd0139c511aa71ca929eecbedef9e5194876135ac08",
    "expected": 2324,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "bbdbdf38a2b03e8c79419fd0139c511aa71ca929eecbedef9e5194876135ac08",
    "source_bytes": 279,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P13",
    "name": "loaded_inner_survives_handle_rebind_to_other_referent",
    "group": "positive",
    "category": "value snapshot",
    "source": "fn run()->i32{let a=25;let b=26;let mut handle=&a;let loaded;{let rr=&handle;loaded=*rr;}handle=&b;let oracle_observation0=*loaded;let oracle_observation1=*handle;oracle_observation0*100+oracle_observation1}",
    "full_source": "fn run()->i32{let a=25;let b=26;let mut handle=&a;let loaded;{let rr=&handle;loaded=*rr;}handle=&b;let oracle_observation0=*loaded;let oracle_observation1=*handle;oracle_observation0*100+oracle_observation1}\nfn main(){if run()==2526{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "d6ce5a0a3c30dc35d2adfe9767434ec0b2f8f59a99d2550792626b6924f7fdcc",
    "expected": 2526,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "d6ce5a0a3c30dc35d2adfe9767434ec0b2f8f59a99d2550792626b6924f7fdcc",
    "source_bytes": 276,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P14",
    "name": "referent_write_after_loaded_inner_last_use",
    "group": "positive",
    "category": "inner liveness",
    "source": "fn run()->i32{let mut v=27;let r=&v;let rr=&r;let loaded=*rr;let oracle_observation0=*loaded;v=28;let oracle_observation1=v;oracle_observation0*100+oracle_observation1}",
    "full_source": "fn run()->i32{let mut v=27;let r=&v;let rr=&r;let loaded=*rr;let oracle_observation0=*loaded;v=28;let oracle_observation1=v;oracle_observation0*100+oracle_observation1}\nfn main(){if run()==2728{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "7965081046c18fe6bfbc1546790ac647b5d76d9399ed550abb67552a7d9ddb4d",
    "expected": 2728,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "7965081046c18fe6bfbc1546790ac647b5d76d9399ed550abb67552a7d9ddb4d",
    "source_bytes": 237,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P15",
    "name": "handle_rebind_after_outer_last_use_while_loaded_inner_lives",
    "group": "positive",
    "category": "layer-specific last-use",
    "source": "fn run()->i32{let a=29;let b=30;let mut r=&a;let rr=&r;let loaded=*rr;r=&b;let oracle_observation0=*loaded;let oracle_observation1=*r;oracle_observation0*100+oracle_observation1}",
    "full_source": "fn run()->i32{let a=29;let b=30;let mut r=&a;let rr=&r;let loaded=*rr;r=&b;let oracle_observation0=*loaded;let oracle_observation1=*r;oracle_observation0*100+oracle_observation1}\nfn main(){if run()==2930{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "ee60e56814cdd9bae7cc6db8b3b069b550b7568c4ccdb8951b26e95ae40c9c53",
    "expected": 2930,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "ee60e56814cdd9bae7cc6db8b3b069b550b7568c4ccdb8951b26e95ae40c9c53",
    "source_bytes": 247,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "P16",
    "name": "T_U_concrete_coincidence_preserves_inner_candidate_authority",
    "group": "positive",
    "category": "declaration authority",
    "source": "fn choose<'o,'i,'p,'j,T,U>(x:&'o &'i T,y:&'p &'j U)->&'i T{*x}\nfn run()->i32{let a=31;let b=32;let ra=&a;let rb=&b;let oracle_observation0=*choose::<i32,i32>(&ra,&rb);oracle_observation0}",
    "full_source": "fn choose<'o,'i,'p,'j,T,U>(x:&'o &'i T,y:&'p &'j U)->&'i T{*x}\nfn run()->i32{let a=31;let b=32;let ra=&a;let rb=&b;let oracle_observation0=*choose::<i32,i32>(&ra,&rb);oracle_observation0}\nfn main(){if run()==31{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "16b62fad6da4ceea4dea00a8e4083ae2dc209593accbbcead117341ef9bde03f",
    "expected": 31,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "16b62fad6da4ceea4dea00a8e4083ae2dc209593accbbcead117341ef9bde03f",
    "source_bytes": 254,
    "proposed_group": "positive",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "N17",
    "name": "loaded_inner_escapes_original_referent_storage",
    "group": "invalid",
    "category": "original referent death",
    "source": "fn run()->i32{let out;{let v=1;let r=&v;let rr=&r;out=*rr;}let oracle_observation0=*out;oracle_observation0}",
    "full_source": "fn run()->i32{let out;{let v=1;let r=&v;let rr=&r;out=*rr;}let oracle_observation0=*out;oracle_observation0}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    "stdout": "",
    "status": "invalid",
    "sha256": "3d5f1f6ef7e0c17eccb126fc91f6b392c2129969b4434e9e9a33506b39669311",
    "expected": 0,
    "expected_stdout": "",
    "source_sha256": "3d5f1f6ef7e0c17eccb126fc91f6b392c2129969b4434e9e9a33506b39669311",
    "source_bytes": 174,
    "proposed_group": "invalid",
    "classification_observed": true,
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
    "id": "N18",
    "name": "returned_outer_escapes_local_inner_handle_storage",
    "group": "invalid",
    "category": "outer dangling reference",
    "source": "fn bad<'i>(x:&'i i32)->&'i &'i i32{let r=x;&r}\nfn run()->i32{0}",
    "full_source": "fn bad<'i>(x:&'i i32)->&'i &'i i32{let r=x;&r}\nfn run()->i32{0}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    "stdout": "",
    "status": "invalid",
    "sha256": "e1712933cf173ba6bc02397f1309af759de683e91135b2e72b45bf3dfaa661ed",
    "expected": 0,
    "expected_stdout": "",
    "source_sha256": "e1712933cf173ba6bc02397f1309af759de683e91135b2e72b45bf3dfaa661ed",
    "source_bytes": 129,
    "proposed_group": "invalid",
    "classification_observed": true,
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
        "message": "cannot return reference to local variable `r`"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0515",
        "message": "cannot return reference to local variable `r`"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N19",
    "name": "live_loaded_inner_blocks_original_referent_write",
    "group": "invalid",
    "category": "write conflict",
    "source": "fn run()->i32{let mut v=1;let r=&v;let rr=&r;let loaded=*rr;v=2;let oracle_observation0=*loaded;oracle_observation0}",
    "full_source": "fn run()->i32{let mut v=1;let r=&v;let rr=&r;let loaded=*rr;v=2;let oracle_observation0=*loaded;oracle_observation0}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    "stdout": "",
    "status": "invalid",
    "sha256": "beb7ef51a0b5024e817633d53ea50fab6894f129b8a4b89f17f22c7d86f3cde6",
    "expected": 0,
    "expected_stdout": "",
    "source_sha256": "beb7ef51a0b5024e817633d53ea50fab6894f129b8a4b89f17f22c7d86f3cde6",
    "source_bytes": 182,
    "proposed_group": "invalid",
    "classification_observed": true,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0506"
    ],
    "stock_check_error_codes": [
      "E0506"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0506",
        "message": "cannot assign to `v` because it is borrowed"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0506",
        "message": "cannot assign to `v` because it is borrowed"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N20",
    "name": "live_loaded_inner_blocks_original_referent_move",
    "group": "invalid",
    "category": "non-Copy referent move conflict",
    "source": "struct Token{v:i32}\nfn take(x:Token)->i32{x.v}\nfn run()->i32{let v=Token{v:1};let r=&v;let rr=&r;let loaded=*rr;let consumed=take(v);let oracle_observation0=loaded.v;let oracle_observation1=consumed;oracle_observation0*100+oracle_observation1}",
    "full_source": "struct Token{v:i32}\nfn take(x:Token)->i32{x.v}\nfn run()->i32{let v=Token{v:1};let r=&v;let rr=&r;let loaded=*rr;let consumed=take(v);let oracle_observation0=loaded.v;let oracle_observation1=consumed;oracle_observation0*100+oracle_observation1}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    "stdout": "",
    "status": "invalid",
    "sha256": "4ab7c90cb5e293fac00f604941a5d9922d084b77cb34256feb305fe359219b83",
    "expected": 0,
    "expected_stdout": "",
    "source_sha256": "4ab7c90cb5e293fac00f604941a5d9922d084b77cb34256feb305fe359219b83",
    "source_bytes": 309,
    "proposed_group": "invalid",
    "classification_observed": true,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0505"
    ],
    "stock_check_error_codes": [
      "E0505"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0505",
        "message": "cannot move out of `v` because it is borrowed"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0505",
        "message": "cannot move out of `v` because it is borrowed"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N21",
    "name": "live_loaded_inner_blocks_original_referent_exclusive_borrow",
    "group": "invalid",
    "category": "mutable borrow conflict",
    "source": "fn write(x:&mut i32){*x=2;}\nfn run()->i32{let mut v=1;let r=&v;let rr=&r;let loaded=*rr;write(&mut v);let oracle_observation0=*loaded;oracle_observation0}",
    "full_source": "fn write(x:&mut i32){*x=2;}\nfn run()->i32{let mut v=1;let r=&v;let rr=&r;let loaded=*rr;write(&mut v);let oracle_observation0=*loaded;oracle_observation0}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    "stdout": "",
    "status": "invalid",
    "sha256": "3eaea23184d59d473889b84b67c14188357329c70557571c0f3c61c906e27cca",
    "expected": 0,
    "expected_stdout": "",
    "source_sha256": "3eaea23184d59d473889b84b67c14188357329c70557571c0f3c61c906e27cca",
    "source_bytes": 220,
    "proposed_group": "invalid",
    "classification_observed": true,
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
        "message": "cannot borrow `v` as mutable because it is also borrowed as immutable"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0502",
        "message": "cannot borrow `v` as mutable because it is also borrowed as immutable"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N22",
    "name": "live_outer_blocks_inner_handle_rebind",
    "group": "invalid",
    "category": "outer access conflict",
    "source": "fn run()->i32{let a=1;let b=2;let mut r=&a;let rr=&r;r=&b;let oracle_observation0=**rr;oracle_observation0}",
    "full_source": "fn run()->i32{let a=1;let b=2;let mut r=&a;let rr=&r;r=&b;let oracle_observation0=**rr;oracle_observation0}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    "stdout": "",
    "status": "invalid",
    "sha256": "cc1abfee3ce816246a46ea906d37dca5eab9bad40b2af68bcce01f6803a3951c",
    "expected": 0,
    "expected_stdout": "",
    "source_sha256": "cc1abfee3ce816246a46ea906d37dca5eab9bad40b2af68bcce01f6803a3951c",
    "source_bytes": 173,
    "proposed_group": "invalid",
    "classification_observed": true,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0506"
    ],
    "stock_check_error_codes": [
      "E0506"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0506",
        "message": "cannot assign to `r` because it is borrowed"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0506",
        "message": "cannot assign to `r` because it is borrowed"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N23",
    "name": "uninitialized_outer_carrier_inner_load",
    "group": "invalid",
    "category": "initialization",
    "source": "fn run()->i32{let rr:&&i32;let loaded=*rr;let oracle_observation0=*loaded;oracle_observation0}",
    "full_source": "fn run()->i32{let rr:&&i32;let loaded=*rr;let oracle_observation0=*loaded;oracle_observation0}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    "stdout": "",
    "status": "invalid",
    "sha256": "f7dfde5dc385cfdc0f3cadc55f5e93c1f5a0e447416b3b50aea425b813b2a5c4",
    "expected": 0,
    "expected_stdout": "",
    "source_sha256": "f7dfde5dc385cfdc0f3cadc55f5e93c1f5a0e447416b3b50aea425b813b2a5c4",
    "source_bytes": 160,
    "proposed_group": "invalid",
    "classification_observed": true,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0381"
    ],
    "stock_check_error_codes": [
      "E0381"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0381",
        "message": "used binding `rr` isn't initialized"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0381",
        "message": "used binding `rr` isn't initialized"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N24",
    "name": "joined_inner_retains_expired_branch_origin",
    "group": "invalid",
    "category": "join escape",
    "source": "fn run()->i32{let a=1;let ra=&a;let out;{let b=2;let rb=&b;let rr;if a==1{rr=&ra;}else{rr=&rb;}out=*rr;}let oracle_observation0=*out;oracle_observation0}",
    "full_source": "fn run()->i32{let a=1;let ra=&a;let out;{let b=2;let rb=&b;let rr;if a==1{rr=&ra;}else{rr=&rb;}out=*rr;}let oracle_observation0=*out;oracle_observation0}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    "stdout": "",
    "status": "invalid",
    "sha256": "8f5b26e384c1830cd7b20ca44fef72ed95ea95656cd06e62287391d732ca60aa",
    "expected": 0,
    "expected_stdout": "",
    "source_sha256": "8f5b26e384c1830cd7b20ca44fef72ed95ea95656cd06e62287391d732ca60aa",
    "source_bytes": 219,
    "proposed_group": "invalid",
    "classification_observed": true,
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
        "message": "`b` does not live long enough"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0597",
        "message": "`b` does not live long enough"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N25",
    "name": "loaded_inner_rebind_does_not_release_old_live_origin",
    "group": "invalid",
    "category": "snapshot-origin conflict",
    "source": "fn run()->i32{let mut a=1;let b=2;let ra=&a;let rb=&b;let mut rr=&ra;let old=*rr;rr=&rb;a=3;let oracle_observation0=*old;let oracle_observation1=**rr;oracle_observation0*100+oracle_observation1}",
    "full_source": "fn run()->i32{let mut a=1;let b=2;let ra=&a;let rb=&b;let mut rr=&ra;let old=*rr;rr=&rb;a=3;let oracle_observation0=*old;let oracle_observation1=**rr;oracle_observation0*100+oracle_observation1}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    "stdout": "",
    "status": "invalid",
    "sha256": "cabfa0ffe3c996cc56791fa515dbcf8af9c68f02507a300657c94a2398075a91",
    "expected": 0,
    "expected_stdout": "",
    "source_sha256": "cabfa0ffe3c996cc56791fa515dbcf8af9c68f02507a300657c94a2398075a91",
    "source_bytes": 260,
    "proposed_group": "invalid",
    "classification_observed": true,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0506"
    ],
    "stock_check_error_codes": [
      "E0506"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0506",
        "message": "cannot assign to `a` because it is borrowed"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0506",
        "message": "cannot assign to `a` because it is borrowed"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "N26",
    "name": "unused_generic_body_returns_inner_under_unrelated_result_region",
    "group": "invalid",
    "category": "all-body region authority",
    "source": "fn bad<'o,'i,'unrelated,T>(x:&'o &'i T)->&'unrelated T{*x}\nfn run()->i32{0}",
    "full_source": "fn bad<'o,'i,'unrelated,T>(x:&'o &'i T)->&'unrelated T{*x}\nfn run()->i32{0}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    "stdout": "",
    "status": "invalid",
    "sha256": "061a9a6179fd36020249952ee5a945e852588f69cebbf41ea3c9848bf20497a4",
    "expected": 0,
    "expected_stdout": "",
    "source_sha256": "061a9a6179fd36020249952ee5a945e852588f69cebbf41ea3c9848bf20497a4",
    "source_bytes": 141,
    "proposed_group": "invalid",
    "classification_observed": true,
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
    "name": "implied_inner_outlives_outer_does_not_prove_reverse",
    "group": "invalid",
    "category": "WF direction stronger return demand",
    "source": "fn bad<'o,'i,T>(x:&'o &'i T)->&'i &'i T{x}\nfn run()->i32{0}",
    "full_source": "fn bad<'o,'i,T>(x:&'o &'i T)->&'i &'i T{x}\nfn run()->i32{0}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    "stdout": "",
    "status": "invalid",
    "sha256": "58ede58052e422df7988a18eb6213d749659121b11a2c44d0b461e6f5a4f014b",
    "expected": 0,
    "expected_stdout": "",
    "source_sha256": "58ede58052e422df7988a18eb6213d749659121b11a2c44d0b461e6f5a4f014b",
    "source_bytes": 125,
    "proposed_group": "invalid",
    "classification_observed": true,
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
    "name": "wrong_inner_position_candidate_despite_T_U_coincidence",
    "group": "invalid",
    "category": "rigid positional type authority",
    "source": "fn bad<'o,'i,'p,'j,T,U>(x:&'o &'i T,y:&'p &'j U)->&'i T{*y}\nfn run()->i32{let a=1;let b=2;let ra=&a;let rb=&b;let _=bad::<i32,i32>(&ra,&rb);0}",
    "full_source": "fn bad<'o,'i,'p,'j,T,U>(x:&'o &'i T,y:&'p &'j U)->&'i T{*y}\nfn run()->i32{let a=1;let b=2;let ra=&a;let rb=&b;let _=bad::<i32,i32>(&ra,&rb);0}\nfn main(){if run()==0{println!(\"nested references oracle ok\");}}\n",
    "stdout": "",
    "status": "invalid",
    "sha256": "61aaf0f369d662e156368cf37217ac25da12defcb2cfa28cee00cf26dbcc82fb",
    "expected": 0,
    "expected_stdout": "",
    "source_sha256": "61aaf0f369d662e156368cf37217ac25da12defcb2cfa28cee00cf26dbcc82fb",
    "source_bytes": 208,
    "proposed_group": "invalid",
    "classification_observed": true,
    "stock_compile_status": 1,
    "stock_check_status": 1,
    "stock_error_codes": [
      "E0308"
    ],
    "stock_check_error_codes": [
      "E0308"
    ],
    "stock_error_diagnostics": [
      {
        "level": "error",
        "code": "E0308",
        "message": "mismatched types"
      }
    ],
    "stock_check_error_diagnostics": [
      {
        "level": "error",
        "code": "E0308",
        "message": "mismatched types"
      }
    ],
    "stock_runtime_status": null,
    "stock_runtime_stdout": null,
    "stock_runtime_stderr": null
  },
  {
    "id": "X29",
    "name": "nested_mutable_reference_chain",
    "group": "legal-excluded",
    "category": "any mutable nested link",
    "source": "fn run()->i32{let mut v=41;let r=&mut v;let rr=&r;let oracle_observation0=**rr;oracle_observation0}",
    "full_source": "fn run()->i32{let mut v=41;let r=&mut v;let rr=&r;let oracle_observation0=**rr;oracle_observation0}\nfn main(){if run()==41{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "0cd40686bbbe209e74df6e40de6bac1084f49ecf6f7cca7efa352b5875cba99d",
    "expected": 41,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "0cd40686bbbe209e74df6e40de6bac1084f49ecf6f7cca7efa352b5875cba99d",
    "source_bytes": 166,
    "proposed_group": "legal-excluded",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "X30",
    "name": "aggregate_reference_fields",
    "group": "legal-excluded",
    "category": "reference-containing aggregate",
    "source": "struct Holder<'a>{r:&'a i32}\nfn run()->i32{let v=42;let h=Holder{r:&v};let oracle_observation0=*h.r;oracle_observation0}",
    "full_source": "struct Holder<'a>{r:&'a i32}\nfn run()->i32{let v=42;let h=Holder{r:&v};let oracle_observation0=*h.r;oracle_observation0}\nfn main(){if run()==42{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "502c3ee736ac7be89edde1b63cc0306067e1797467fa27529848433d91529a9f",
    "expected": 42,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "502c3ee736ac7be89edde1b63cc0306067e1797467fa27529848433d91529a9f",
    "source_bytes": 187,
    "proposed_group": "legal-excluded",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "X31",
    "name": "ordinary_reference_generic_actual",
    "group": "legal-excluded",
    "category": "source reference type actual",
    "source": "fn id<T>(x:T)->T{x}\nfn run()->i32{let v=43;let r=id::<&i32>(&v);let oracle_observation0=*r;oracle_observation0}",
    "full_source": "fn id<T>(x:T)->T{x}\nfn run()->i32{let v=43;let r=id::<&i32>(&v);let oracle_observation0=*r;oracle_observation0}\nfn main(){if run()==43{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "80ae5b278b3d3fb1165e17e54bc23ffbc9238019e0b347464035f08a6fc95070",
    "expected": 43,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "80ae5b278b3d3fb1165e17e54bc23ffbc9238019e0b347464035f08a6fc95070",
    "source_bytes": 178,
    "proposed_group": "legal-excluded",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  },
  {
    "id": "X32",
    "name": "shared_reference_depth_greater_than_two",
    "group": "legal-excluded",
    "category": "depth boundary",
    "source": "fn run()->i32{let v=44;let r=&v;let rr=&r;let rrr=&rr;let oracle_observation0=***rrr;oracle_observation0}",
    "full_source": "fn run()->i32{let v=44;let r=&v;let rr=&r;let rrr=&rr;let oracle_observation0=***rrr;oracle_observation0}\nfn main(){if run()==44{println!(\"nested references oracle ok\");}}\n",
    "stdout": "nested references oracle ok\n",
    "status": "valid",
    "sha256": "677de66b1201fd9d44cc3a8b133897b4076c0a5c36ded40a57a877877c1d82b0",
    "expected": 44,
    "expected_stdout": "nested references oracle ok\n",
    "source_sha256": "677de66b1201fd9d44cc3a8b133897b4076c0a5c36ded40a57a877877c1d82b0",
    "source_bytes": 172,
    "proposed_group": "legal-excluded",
    "classification_observed": true,
    "stock_compile_status": 0,
    "stock_check_status": 0,
    "stock_error_codes": [],
    "stock_check_error_codes": [],
    "stock_error_diagnostics": [],
    "stock_check_error_diagnostics": [],
    "stock_runtime_status": 0,
    "stock_runtime_stdout": "nested references oracle ok\n",
    "stock_runtime_stderr": ""
  }
]
'''
FIXTURES = json.loads(FROZEN_CORPUS_JSON)

def corpus():
    if hashlib.sha256(FROZEN_CORPUS_JSON.encode()).hexdigest() != ACCEPTANCE_CORPUS_SHA256:
        raise ValueError("frozen nested reference dictionary bytes changed")
    kinds = {"positive": "valid", "invalid": "invalid", "legal-excluded": "boundary"}
    if len(FIXTURES) != 32 or len({f["name"] for f in FIXTURES}) != 32:
        raise ValueError("expected exactly32 unique nested reference fixtures")
    if [sum(f["group"] == g for f in FIXTURES) for g in kinds] != [16, 12, 4]:
        raise ValueError("frozen nested reference classifications changed")
    for fixture in FIXTURES:
        source = fixture["full_source"]
        if (hashlib.sha256(source.encode()).hexdigest() != fixture["source_sha256"]
                or fixture["sha256"] != fixture["source_sha256"]
                or len(source.encode()) != fixture["source_bytes"]):
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
        if fixture["id"] in ("N26", "N27"):
            required = [{"level": "error", "code": None,
                         "message": "lifetime may not live long enough"}]
            if build_errors != required or check_errors != required:
                row["failures"].append("N26/N27 require exact structured code-less lifetime diagnostic")
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
            prefix="jr-nested-references-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "nested-references-v1",
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
                raise SystemExit(f"nested reference discrepancy, frozen sources not changed: {args.output}")
        record({"record": "summary", **totals, "passed": totals["failure"] == 0})


if __name__ == "__main__":
    main()