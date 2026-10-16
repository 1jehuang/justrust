#!/usr/bin/env python3
"""Bounded builtin compound assignment oracle, not full Rust compatibility.

Exact stock-classified sources/stdout mirror tests/compound_assignments.rs.
Reuse shared Case/run_case/process and bounded capture with supplied native
workers1/8 BUILD/CHECK. No native binary is built by this harness. Ordinary
cases retain strict status/stdout/stderr comparison. Only eight exact declared
source-bound runtime_error fixtures admit completed101/exactstdout and differing
native panic stderr. Stock panic marker and fixture reason remain mandatory.
No signals, timeouts, truncation, compiler crashes or metadata overrides pass.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time

from module_correctness import Case, accepted, completed, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "f5bdf3fcfdb06d3d4771f16b0e5db3342e660bc9bce6295036968475965b8b2a"
TRAP_POLICY = ("declared runtime_error only: compile/check0, executable required, "
               "completed runtime101 and exact stdout; stock stderr contains panicked at "
               "and fixture reason; native stderr captured but may differ or be empty; "
               "signals/timeouts/truncation/compiler crashes fail")

# Frozen bear oracle. full_source is authoritative, without additional wrappers.
FIXTURES = json.loads(r'''
[
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "struct S{x:i8}fn apply(p:&mut i8){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "all_operations_i8",
    "source": "struct S{x:i8}fn apply(p:&mut i8){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "struct S{x:i16}fn apply(p:&mut i16){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "all_operations_i16",
    "source": "struct S{x:i16}fn apply(p:&mut i16){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "struct S{x:i32}fn apply(p:&mut i32){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "all_operations_i32",
    "source": "struct S{x:i32}fn apply(p:&mut i32){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "struct S{x:i64}fn apply(p:&mut i64){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "all_operations_i64",
    "source": "struct S{x:i64}fn apply(p:&mut i64){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "struct S{x:isize}fn apply(p:&mut isize){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "all_operations_isize",
    "source": "struct S{x:isize}fn apply(p:&mut isize){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "struct S{x:u8}fn apply(p:&mut u8){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "all_operations_u8",
    "source": "struct S{x:u8}fn apply(p:&mut u8){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "struct S{x:u16}fn apply(p:&mut u16){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "all_operations_u16",
    "source": "struct S{x:u16}fn apply(p:&mut u16){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "struct S{x:u32}fn apply(p:&mut u32){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "all_operations_u32",
    "source": "struct S{x:u32}fn apply(p:&mut u32){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "struct S{x:u64}fn apply(p:&mut u64){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "all_operations_u64",
    "source": "struct S{x:u64}fn apply(p:&mut u64){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "struct S{x:usize}fn apply(p:&mut usize){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "all_operations_usize",
    "source": "struct S{x:usize}fn apply(p:&mut usize){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "fn run()->i32{let mut x=2;x+={x=10;1};let mut y:i32;y+={y=20;2};if x==11 && y==22{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "rhs_replaces_and_initializes_local",
    "source": "fn run()->i32{let mut x=2;x+={x=10;1};let mut y:i32;y+={y=20;2};if x==11 && y==22{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "fn run()->i32{let mut x=2;let mut y=10;let mut p=&mut x;*p+={p=&mut y;3};if x==2 && y==13{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "rhs_retargets_deref",
    "source": "fn run()->i32{let mut x=2;let mut y=10;let mut p=&mut x;*p+={p=&mut y;3};if x==2 && y==13{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "fn run()->i32{let mut a=[2,10];let mut n=0;a[{n=n*10+1;0usize}]+={n=n*10+2;3};if n==21 && a[0]==5 && a[1]==10{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "dynamic_index_rhs_first_once",
    "source": "fn run()->i32{let mut a=[2,10];let mut n=0;a[{n=n*10+1;0usize}]+={n=n*10+2;3};if n==21 && a[0]==5 && a[1]==10{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "fn run()->i32{let mut a=[2];a[0]+={a[0]=10;1};if a[0]==11{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "rhs_replaces_same_element",
    "source": "fn run()->i32{let mut a=[2];a[0]+={a[0]=10;1};if a[0]==11{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "fn run()->i32{let mut n=0;let mut x=false;x&={n=n+1;true};let mut y=true;y|={n=n+1;false};let mut z=true;z^={n=n+1;true};if n==3 && !x && y && !z{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "boolean_eager_compounds",
    "source": "fn run()->i32{let mut n=0;let mut x=false;x&={n=n+1;true};let mut y=true;y|={n=n+1;false};let mut z=true;z^={n=n+1;true};if n==3 && !x && y && !z{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "fn run()->i32{let mut x=3u8;x<<=2u64;x>>=1i16;if x==6u8{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "positive",
    "name": "heterogeneous_shift_compounds",
    "source": "fn run()->i32{let mut x=3u8;x<<=2u64;x>>=1i16;if x==6u8{7}else{0}}"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(a:i8,y:i8)->i8{let mut x=a;x+=y;x}\nfn main(){println!(\"before\");let x=op(127i8,1i8);println!(\"after\");}",
    "group": "runtime_error",
    "name": "add_overflow",
    "source": "fn op(a:i8,y:i8)->i8{let mut x=a;x+=y;x}",
    "stock_panic_reason": "attempt to add with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(a:u8,y:u8)->u8{let mut x=a;x-=y;x}\nfn main(){println!(\"before\");let x=op(0u8,1u8);println!(\"after\");}",
    "group": "runtime_error",
    "name": "subtract_overflow",
    "source": "fn op(a:u8,y:u8)->u8{let mut x=a;x-=y;x}",
    "stock_panic_reason": "attempt to subtract with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(a:i16,y:i16)->i16{let mut x=a;x*=y;x}\nfn main(){println!(\"before\");let x=op(32767i16,2i16);println!(\"after\");}",
    "group": "runtime_error",
    "name": "multiply_overflow",
    "source": "fn op(a:i16,y:i16)->i16{let mut x=a;x*=y;x}",
    "stock_panic_reason": "attempt to multiply with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(a:i32,y:i32)->i32{let mut x=a;x/=y;x}\nfn main(){println!(\"before\");let x=op(7i32,0i32);println!(\"after\");}",
    "group": "runtime_error",
    "name": "divide_zero",
    "source": "fn op(a:i32,y:i32)->i32{let mut x=a;x/=y;x}",
    "stock_panic_reason": "attempt to divide by zero"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(a:u64,y:u64)->u64{let mut x=a;x%=y;x}\nfn main(){println!(\"before\");let x=op(7u64,0u64);println!(\"after\");}",
    "group": "runtime_error",
    "name": "remainder_zero",
    "source": "fn op(a:u64,y:u64)->u64{let mut x=a;x%=y;x}",
    "stock_panic_reason": "attempt to calculate the remainder with a divisor of zero"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(a:i64,y:i64)->i64{let mut x=a;x/=y;x}\nfn main(){println!(\"before\");let x=op(-9223372036854775808i64,-1i64);println!(\"after\");}",
    "group": "runtime_error",
    "name": "minimum_divide_negative_one",
    "source": "fn op(a:i64,y:i64)->i64{let mut x=a;x/=y;x}",
    "stock_panic_reason": "attempt to divide with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(a:u8,y:i8)->u8{let mut x=a;x<<=y;x}\nfn main(){println!(\"before\");let x=op(1u8,-1i8);println!(\"after\");}",
    "group": "runtime_error",
    "name": "negative_shift_count",
    "source": "fn op(a:u8,y:i8)->u8{let mut x=a;x<<=y;x}",
    "stock_panic_reason": "attempt to shift left with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(a:u64,y:u64)->u64{let mut x=a;x>>=y;x}\nfn main(){println!(\"before\");let x=op(1u64,64u64);println!(\"after\");}",
    "group": "runtime_error",
    "name": "width_shift_count",
    "source": "fn op(a:u64,y:u64)->u64{let mut x=a;x>>=y;x}",
    "stock_panic_reason": "attempt to shift right with overflow"
  },
  {
    "expected": "E0384",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=1;x+=2;x}\nfn main(){run();}",
    "group": "negative",
    "name": "immutable_target",
    "source": "fn run()->i32{let x=1;x+=2;x}"
  },
  {
    "expected": "E0381",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let mut x:i32;x+=1;x}\nfn main(){run();}",
    "group": "negative",
    "name": "uninitialized_target_read",
    "source": "fn run()->i32{let mut x:i32;x+=1;x}"
  },
  {
    "expected": "E0594",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=1;let p=&x;*p+=1;7}\nfn main(){run();}",
    "group": "negative",
    "name": "shared_reference_write",
    "source": "fn run()->i32{let x=1;let p=&x;*p+=1;7}"
  },
  {
    "expected": "E0506",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let mut x=1;let r=&x;x+=1;*r}\nfn main(){run();}",
    "group": "negative",
    "name": "conflicting_live_loan",
    "source": "fn run()->i32{let mut x=1;let r=&x;x+=1;*r}"
  },
  {
    "expected": "E0308",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let mut x=1u8;x+=2u16;7}\nfn main(){run();}",
    "group": "negative",
    "name": "mixed_nonshift_integer_types",
    "source": "fn run()->i32{let mut x=1u8;x+=2u16;7}"
  },
  {
    "expected": "E0368",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let mut x=true;x+=false;7}\nfn main(){run();}",
    "group": "negative",
    "name": "boolean_arithmetic",
    "source": "fn run()->i32{let mut x=true;x+=false;7}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "fn run()->i32{let mut x=3u128;x+=4u128;x<<=2u8;x>>=2u8;if x==7u128{7}else{0}}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "legalunsupported",
    "name": "integer128_compounds",
    "source": "fn run()->i32{let mut x=3u128;x+=4u128;x<<=2u8;x>>=2u8;if x==7u128{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "compound oracle ok\n",
    "full_source": "struct S(i32);impl std::ops::AddAssign<i32> for S{fn add_assign(&mut self,r:i32){self.0=self.0+r;}}fn run()->i32{let mut s=S(3);s+=4;s.0}\nfn main(){if run()==7{println!(\"compound oracle ok\");}}",
    "group": "legalunsupported",
    "name": "custom_trait_compound",
    "source": "struct S(i32);impl std::ops::AddAssign<i32> for S{fn add_assign(&mut self,r:i32){self.0=self.0+r;}}fn run()->i32{let mut s=S(3);s+=4;s.0}"
  }
]
''')
TRAPS = {f["name"]: f for f in FIXTURES if f["group"] == "runtime_error"}
assert len(TRAPS) == 8
assert all(f["expected"] == 101 and f["stock_panic_reason"]
           and "expected_stdout" in f for f in TRAPS.values())


def corpus():
    kinds = {"positive": "valid", "negative": "invalid",
             "legalunsupported": "boundary", "runtime_error": "accepted_trap101"}
    for fixture in FIXTURES:
        yield Case(fixture["name"], kinds[fixture["group"]], fixture["full_source"],
                   fixture.get("expected_stdout", ""))


def trap_result(case, result, *, stock):
    """No inference of traps from failures: only exact declared fixtures qualify."""
    fixture = TRAPS.get(case.name)
    if (fixture is None or case.kind != "accepted_trap101"
            or case.source != fixture["full_source"] or case.stdout != fixture["expected_stdout"]):
        return False
    if (not completed(result) or result["status"] != 101
            or result["stdout"] != case.stdout
            or result["stdout_sha256"] != hashlib.sha256(case.stdout.encode()).hexdigest()):
        return False
    return (not stock or ("panicked at" in result["stderr"]
                          and fixture["stock_panic_reason"] in result["stderr"]))


def stock_trap_case(case, index, directory, rustc, compile_timeout, run_timeout):
    directory.mkdir()
    source = directory / "input.rs"
    source.write_text(case.source, encoding="utf-8")
    output = directory / "stock"
    metadata = directory / "stock.rmeta"
    assert not output.exists() and not metadata.exists()
    argv = [rustc, "--edition=2024", "--crate-name", "reference", "-C",
            "overflow-checks=yes", "-D", "overflowing_literals", "-D", "arithmetic_overflow", source]
    compile_result = process([*argv, "-o", output], directory, compile_timeout)
    row = {"record": "stock_case", "index": index, "name": case.name,
           "kind": case.kind, "source": case.source,
           "source_sha256": hashlib.sha256(case.source.encode()).hexdigest(),
           "stock_compile": compile_result, "failures": []}
    if not accepted(compile_result) or not output.is_file():
        row["failures"].append("declared trap must compile with stock and produce executable")
    else:
        row["stock_run"] = process([output], directory, run_timeout)
        if not trap_result(case, row["stock_run"], stock=True):
            row["failures"].append("stock trap must complete101 with exact stdout and panic reason")
    row["stock_check"] = process([*argv, "--emit=metadata", "-o", metadata],
                                  directory, compile_timeout)
    if not accepted(row["stock_check"]) or not metadata.is_file():
        row["failures"].append("declared trap stock check must succeed with metadata")
    row["passed"] = not row["failures"]
    row["outcome"] = "stock_classified" if row["passed"] else "failure"
    return row


def run_trap_case(case, index, directory, binary, rustc, compile_timeout, run_timeout):
    start = time.perf_counter_ns()
    row = stock_trap_case(case, index, directory, rustc, compile_timeout, run_timeout)
    row["record"] = "case"
    row["workers"] = []
    source = directory / "input.rs"
    for workers in (1, 8):
        output = directory / f"native-{workers}"
        assert not output.exists()
        compile_result = process([binary, source, "-o", output, "--workers", workers],
                                 directory, compile_timeout)
        worker = {"workers": workers, "compile": compile_result,
                  "output_exists": output.exists()}
        row["workers"].append(worker)
        if not accepted(compile_result) or not output.is_file():
            row["failures"].append(f"trap fixture must compile normally with workers={workers}")
        else:
            worker["run"] = process([output], directory, run_timeout)
            if not trap_result(case, worker["run"], stock=False):
                row["failures"].append(f"native trap must complete101/exactstdout with workers={workers}")
        before_check = set(directory.iterdir())
        worker["check"] = process([binary, source, "--workers", workers, "--check"],
                                   directory, compile_timeout)
        if not accepted(worker["check"]) or set(directory.iterdir()) != before_check:
            row["failures"].append(f"trap check must succeed without artifacts with workers={workers}")
    row["passed"] = not row["failures"]
    row["outcome"] = "accepted_trap101" if row["passed"] else "failure"
    row["wall_ns"] = time.perf_counter_ns() - start
    return row


def native_case(case, *args):
    if case.kind == "accepted_trap101":
        return run_trap_case(case, *args)
    row = run_case(case, *args)
    return stock_negative_phases(case, row, args[1], args[3], args[4])


def stock_case(case, *args):
    if case.kind == "accepted_trap101":
        return stock_trap_case(case, *args)
    row = ordinary_stock_case(case, *args)
    return stock_negative_phases(case, row, args[1], args[2], args[3])


FIXTURE_BY_NAME = {fixture["name"]: fixture for fixture in FIXTURES}
assert all("stock_check_status" not in fixture for fixture in FIXTURES)


def stock_negative_phases(case, row, directory, rustc, compile_timeout):
    """Every invalid fixture must fail stock BUILD and metadata without artifacts.

    Native BUILD/CHECK rejection remains enforced by shared run_case.
    """
    if case.kind != "invalid":
        return row
    fixture = FIXTURE_BY_NAME[case.name]
    assert case.source == fixture["full_source"]
    metadata = directory / "stock-invalid.rmeta"
    assert not metadata.exists()
    before_check = set(directory.iterdir())
    row["stock_check"] = process(
        [rustc, "--edition=2024", "--crate-name", "reference", "-C",
         "overflow-checks=yes", "-D", "overflowing_literals", "-D", "arithmetic_overflow",
         directory / "input.rs", "--emit=metadata", "-o", metadata],
        directory, compile_timeout)
    expected_status = 1
    row["expected_stock_check_status"] = expected_status
    result = row["stock_check"]
    phase_ok = (rejected(result) and not metadata.exists()
                and set(directory.iterdir()) == before_check)
    if not phase_ok:
        row["failures"].append("stock metadata status/artifact disagrees with frozen phase policy")
    code = fixture["expected"]
    diagnostic = row["stock_compile"]["stderr"]
    matched = (f"error[{code}]" in diagnostic if code.startswith("E")
               else code in diagnostic or code.replace("_", "-") in diagnostic)
    if not matched:
        row["failures"].append("stock BUILD diagnostic disagrees with frozen classification")
    row["passed"] = not row["failures"]
    row["outcome"] = ("stock_classified" if row["record"] == "stock_case"
                      else "invalid_rejected") if row["passed"] else "failure"
    return row


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
    assert len(cases) == 32 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-compound-assignments-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "compound-assignments-v1",
                "case_count": len(cases),
                "case_counts": {kind: sum(c.kind == kind for c in cases)
                                for kind in ("valid", "invalid", "boundary", "accepted_trap101")},
                "stock_only": args.stock_only,
                "workers": [] if args.stock_only else [1, 8],
                "modes": [] if args.stock_only else ["build", "check"], "edition": 2024,
                "reference_modes": ["build", "check_invalid", "check_trap", "run"], "rustc_path": str(rustc),
                "reference": version["stdout"],
                "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest() if binary else None,
                "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                "shared_harness_sha256": hashlib.sha256(Path(__file__).with_name("module_correctness.py").read_bytes()).hexdigest(),
                "corpus_sha256": corpus_hash,
                "acceptance_corpus_sha256": ACCEPTANCE_CORPUS_SHA256,
                "trap_policy": TRAP_POLICY,
                "timing_scope": "process/deadline/capture overhead included, not a compiler latency benchmark",
                "note": "legal unsupported boundaries are not invalid Rust or parity"})
        totals = {"accepted_parity": 0, "invalid_rejected": 0, "unsupported_boundary": 0,
                  "stock_classified": 0, "accepted_trap101": 0, "failure": 0}
        for index, case in enumerate(cases):
            if args.stock_only:
                row = stock_case(case, index, directory / str(index), rustc,
                                 args.compile_timeout, args.run_timeout)
            else:
                row = native_case(case, index, directory / str(index), binary, rustc,
                               args.compile_timeout, args.run_timeout)
            row["expected_stdout"] = case.stdout
            row["expected_exit"] = 101 if case.kind == "accepted_trap101" else (None if case.kind == "invalid" else 0)
            if case.kind == "accepted_trap101":
                row["runtime_policy"] = TRAP_POLICY
                row["stock_panic_reason"] = TRAPS[case.name]["stock_panic_reason"]
            record(row)
            totals[row["outcome"]] += 1
            print(f"{case.name}: {row['outcome']}", flush=True)
        record({"record": "summary", **totals, "passed": totals["failure"] == 0})
        if totals["failure"]:
            raise SystemExit(f"{totals['failure']} compound assignment discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
