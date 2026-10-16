#!/usr/bin/env python3
"""Bounded <=64-bit integer foundation oracle, not full Rust compatibility.

Exact stock-classified sources/stdout mirror tests/integer_types.rs. Ordinary cases use
unchanged shared Case/run_case/process infrastructure and strict output parity.
Only declared runtime_error fixtures allow native versus stock panic stderr text
to differ: both runs must finish exactly101 with exact stdout, stock panic marker
and reason verified. Signals, timeout, truncation and compiler crashes never pass.
Native binaries are supplied, never built by this harness.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time

from module_correctness import Case, accepted, completed, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "262ec79b43edfab4a73d22d292dc58afb87f5642903978bf685368081a3b6c75"
TRAP_POLICY = ("declared runtime_error only: compile/check0, executable required, "
               "completed runtime101 and exact stdout; stock stderr contains panicked at "
               "and fixture reason; native stderr captured but may differ or be empty; "
               "signals/timeouts/truncation/compiler crashes fail")

# Cast native-pass promotion: integer_cast_syntax is positive; source/stdout/value unchanged.
# Original immutable acceptance artifact hash above is retained.
# Frozen bear oracle. full_source is authoritative, without additional wrappers.
FIXTURES = json.loads(r'''
[
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn id(x:i8)->i8{x} fn run()->i32{let lo:i8=-128i8;let hi:i8=127i8;let grouped:i8=-(((128i8)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:i8=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "i8_bounds_arithmetic_not_transport",
    "source": "fn id(x:i8)->i8{x} fn run()->i32{let lo:i8=-128i8;let hi:i8=127i8;let grouped:i8=-(((128i8)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:i8=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn id(x:i16)->i16{x} fn run()->i32{let lo:i16=-32768i16;let hi:i16=32767i16;let grouped:i16=-(((32768i16)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:i16=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "i16_bounds_arithmetic_not_transport",
    "source": "fn id(x:i16)->i16{x} fn run()->i32{let lo:i16=-32768i16;let hi:i16=32767i16;let grouped:i16=-(((32768i16)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:i16=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn id(x:i32)->i32{x} fn run()->i32{let lo:i32=-2147483648i32;let hi:i32=2147483647i32;let grouped:i32=-(((2147483648i32)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:i32=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "i32_bounds_arithmetic_not_transport",
    "source": "fn id(x:i32)->i32{x} fn run()->i32{let lo:i32=-2147483648i32;let hi:i32=2147483647i32;let grouped:i32=-(((2147483648i32)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:i32=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn id(x:i64)->i64{x} fn run()->i32{let lo:i64=-9223372036854775808i64;let hi:i64=9223372036854775807i64;let grouped:i64=-(((9223372036854775808i64)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:i64=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "i64_bounds_arithmetic_not_transport",
    "source": "fn id(x:i64)->i64{x} fn run()->i32{let lo:i64=-9223372036854775808i64;let hi:i64=9223372036854775807i64;let grouped:i64=-(((9223372036854775808i64)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:i64=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn id(x:isize)->isize{x} fn run()->i32{let lo:isize=-9223372036854775808isize;let hi:isize=9223372036854775807isize;let grouped:isize=-(((9223372036854775808isize)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:isize=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "isize_bounds_arithmetic_not_transport",
    "source": "fn id(x:isize)->isize{x} fn run()->i32{let lo:isize=-9223372036854775808isize;let hi:isize=9223372036854775807isize;let grouped:isize=-(((9223372036854775808isize)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:isize=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn id(x:u8)->u8{x} fn run()->i32{let lo:u8=0u8;let hi:u8=255u8;let grouped:u8=0u8;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:u8=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "u8_bounds_arithmetic_not_transport",
    "source": "fn id(x:u8)->u8{x} fn run()->i32{let lo:u8=0u8;let hi:u8=255u8;let grouped:u8=0u8;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:u8=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn id(x:u16)->u16{x} fn run()->i32{let lo:u16=0u16;let hi:u16=65535u16;let grouped:u16=0u16;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:u16=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "u16_bounds_arithmetic_not_transport",
    "source": "fn id(x:u16)->u16{x} fn run()->i32{let lo:u16=0u16;let hi:u16=65535u16;let grouped:u16=0u16;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:u16=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn id(x:u32)->u32{x} fn run()->i32{let lo:u32=0u32;let hi:u32=4294967295u32;let grouped:u32=0u32;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:u32=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "u32_bounds_arithmetic_not_transport",
    "source": "fn id(x:u32)->u32{x} fn run()->i32{let lo:u32=0u32;let hi:u32=4294967295u32;let grouped:u32=0u32;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:u32=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn id(x:u64)->u64{x} fn run()->i32{let lo:u64=0u64;let hi:u64=18446744073709551615u64;let grouped:u64=0u64;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:u64=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "u64_bounds_arithmetic_not_transport",
    "source": "fn id(x:u64)->u64{x} fn run()->i32{let lo:u64=0u64;let hi:u64=18446744073709551615u64;let grouped:u64=0u64;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:u64=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn id(x:usize)->usize{x} fn run()->i32{let lo:usize=0usize;let hi:usize=18446744073709551615usize;let grouped:usize=0usize;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:usize=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "usize_bounds_arithmetic_not_transport",
    "source": "fn id(x:usize)->usize{x} fn run()->i32{let lo:usize=0usize;let hi:usize=18446744073709551615usize;let grouped:usize=0usize;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:usize=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn run()->i32{let a:u8=0xff_u8;let b:u16=0b1010_0101u16;let c:i16=0o177_i16;let d:i16=0x7fff_i16;let e:u64=18_446_744_073_709_551_615u64;if a==255u8 && b==165u16 && c==127i16 && d==32767i16 && e==18446744073709551615u64{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "radices_and_underscores",
    "source": "fn run()->i32{let a:u8=0xff_u8;let b:u16=0b1010_0101u16;let c:i16=0o177_i16;let d:i16=0x7fff_i16;let e:u64=18_446_744_073_709_551_615u64;if a==255u8 && b==165u16 && c==127i16 && d==32767i16 && e==18446744073709551615u64{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn small(x:i8)->i32{match x{-128i8=>7,127i8=>3,_=>0}} fn wide(x:u64)->i32{match x{18446744073709551615u64=>7,_=>0}} fn run()->i32{if small(-128i8)==7 && wide(18446744073709551615u64)==7{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "typed_integer_literal_patterns",
    "source": "fn small(x:i8)->i32{match x{-128i8=>7,127i8=>3,_=>0}} fn wide(x:u64)->i32{match x{18446744073709551615u64=>7,_=>0}} fn run()->i32{if small(-128i8)==7 && wide(18446744073709551615u64)==7{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn q_i8(x:i8,y:i8)->i8{x/y} fn r_i8(x:i8,y:i8)->i8{x%y} fn q_i16(x:i16,y:i16)->i16{x/y} fn r_i16(x:i16,y:i16)->i16{x%y} fn q_i32(x:i32,y:i32)->i32{x/y} fn r_i32(x:i32,y:i32)->i32{x%y} fn q_i64(x:i64,y:i64)->i64{x/y} fn r_i64(x:i64,y:i64)->i64{x%y} fn q_isize(x:isize,y:isize)->isize{x/y} fn r_isize(x:isize,y:isize)->isize{x%y} fn run()->i32{if q_i8(-7i8,3i8)==-2i8 && r_i8(-7i8,3i8)==-1i8 && q_i16(-7i16,3i16)==-2i16 && r_i16(-7i16,3i16)==-1i16 && q_i32(-7i32,3i32)==-2i32 && r_i32(-7i32,3i32)==-1i32 && q_i64(-7i64,3i64)==-2i64 && r_i64(-7i64,3i64)==-1i64 && q_isize(-7isize,3isize)==-2isize && r_isize(-7isize,3isize)==-1isize{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "signed_division_remainder_signs",
    "source": "fn q_i8(x:i8,y:i8)->i8{x/y} fn r_i8(x:i8,y:i8)->i8{x%y} fn q_i16(x:i16,y:i16)->i16{x/y} fn r_i16(x:i16,y:i16)->i16{x%y} fn q_i32(x:i32,y:i32)->i32{x/y} fn r_i32(x:i32,y:i32)->i32{x%y} fn q_i64(x:i64,y:i64)->i64{x/y} fn r_i64(x:i64,y:i64)->i64{x%y} fn q_isize(x:isize,y:isize)->isize{x/y} fn r_isize(x:isize,y:isize)->isize{x%y} fn run()->i32{if q_i8(-7i8,3i8)==-2i8 && r_i8(-7i8,3i8)==-1i8 && q_i16(-7i16,3i16)==-2i16 && r_i16(-7i16,3i16)==-1i16 && q_i32(-7i32,3i32)==-2i32 && r_i32(-7i32,3i32)==-1i32 && q_i64(-7i64,3i64)==-2i64 && r_i64(-7i64,3i64)==-1i64 && q_isize(-7isize,3isize)==-2isize && r_isize(-7isize,3isize)==-1isize{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn classify(x:u8)->i32{match x{0u8=>7,1u8=>7,2u8=>7,3u8=>7,4u8=>7,5u8=>7,6u8=>7,7u8=>7,8u8=>7,9u8=>7,10u8=>7,11u8=>7,12u8=>7,13u8=>7,14u8=>7,15u8=>7,16u8=>7,17u8=>7,18u8=>7,19u8=>7,20u8=>7,21u8=>7,22u8=>7,23u8=>7,24u8=>7,25u8=>7,26u8=>7,27u8=>7,28u8=>7,29u8=>7,30u8=>7,31u8=>7,32u8=>7,33u8=>7,34u8=>7,35u8=>7,36u8=>7,37u8=>7,38u8=>7,39u8=>7,40u8=>7,41u8=>7,42u8=>7,43u8=>7,44u8=>7,45u8=>7,46u8=>7,47u8=>7,48u8=>7,49u8=>7,50u8=>7,51u8=>7,52u8=>7,53u8=>7,54u8=>7,55u8=>7,56u8=>7,57u8=>7,58u8=>7,59u8=>7,60u8=>7,61u8=>7,62u8=>7,63u8=>7,64u8=>7,65u8=>7,66u8=>7,67u8=>7,68u8=>7,69u8=>7,70u8=>7,71u8=>7,72u8=>7,73u8=>7,74u8=>7,75u8=>7,76u8=>7,77u8=>7,78u8=>7,79u8=>7,80u8=>7,81u8=>7,82u8=>7,83u8=>7,84u8=>7,85u8=>7,86u8=>7,87u8=>7,88u8=>7,89u8=>7,90u8=>7,91u8=>7,92u8=>7,93u8=>7,94u8=>7,95u8=>7,96u8=>7,97u8=>7,98u8=>7,99u8=>7,100u8=>7,101u8=>7,102u8=>7,103u8=>7,104u8=>7,105u8=>7,106u8=>7,107u8=>7,108u8=>7,109u8=>7,110u8=>7,111u8=>7,112u8=>7,113u8=>7,114u8=>7,115u8=>7,116u8=>7,117u8=>7,118u8=>7,119u8=>7,120u8=>7,121u8=>7,122u8=>7,123u8=>7,124u8=>7,125u8=>7,126u8=>7,127u8=>7,128u8=>7,129u8=>7,130u8=>7,131u8=>7,132u8=>7,133u8=>7,134u8=>7,135u8=>7,136u8=>7,137u8=>7,138u8=>7,139u8=>7,140u8=>7,141u8=>7,142u8=>7,143u8=>7,144u8=>7,145u8=>7,146u8=>7,147u8=>7,148u8=>7,149u8=>7,150u8=>7,151u8=>7,152u8=>7,153u8=>7,154u8=>7,155u8=>7,156u8=>7,157u8=>7,158u8=>7,159u8=>7,160u8=>7,161u8=>7,162u8=>7,163u8=>7,164u8=>7,165u8=>7,166u8=>7,167u8=>7,168u8=>7,169u8=>7,170u8=>7,171u8=>7,172u8=>7,173u8=>7,174u8=>7,175u8=>7,176u8=>7,177u8=>7,178u8=>7,179u8=>7,180u8=>7,181u8=>7,182u8=>7,183u8=>7,184u8=>7,185u8=>7,186u8=>7,187u8=>7,188u8=>7,189u8=>7,190u8=>7,191u8=>7,192u8=>7,193u8=>7,194u8=>7,195u8=>7,196u8=>7,197u8=>7,198u8=>7,199u8=>7,200u8=>7,201u8=>7,202u8=>7,203u8=>7,204u8=>7,205u8=>7,206u8=>7,207u8=>7,208u8=>7,209u8=>7,210u8=>7,211u8=>7,212u8=>7,213u8=>7,214u8=>7,215u8=>7,216u8=>7,217u8=>7,218u8=>7,219u8=>7,220u8=>7,221u8=>7,222u8=>7,223u8=>7,224u8=>7,225u8=>7,226u8=>7,227u8=>7,228u8=>7,229u8=>7,230u8=>7,231u8=>7,232u8=>7,233u8=>7,234u8=>7,235u8=>7,236u8=>7,237u8=>7,238u8=>7,239u8=>7,240u8=>7,241u8=>7,242u8=>7,243u8=>7,244u8=>7,245u8=>7,246u8=>7,247u8=>7,248u8=>7,249u8=>7,250u8=>7,251u8=>7,252u8=>7,253u8=>7,254u8=>7,255u8=>7}} fn run()->i32{classify(255u8)}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "exhaustive_u8_literal_match",
    "source": "fn classify(x:u8)->i32{match x{0u8=>7,1u8=>7,2u8=>7,3u8=>7,4u8=>7,5u8=>7,6u8=>7,7u8=>7,8u8=>7,9u8=>7,10u8=>7,11u8=>7,12u8=>7,13u8=>7,14u8=>7,15u8=>7,16u8=>7,17u8=>7,18u8=>7,19u8=>7,20u8=>7,21u8=>7,22u8=>7,23u8=>7,24u8=>7,25u8=>7,26u8=>7,27u8=>7,28u8=>7,29u8=>7,30u8=>7,31u8=>7,32u8=>7,33u8=>7,34u8=>7,35u8=>7,36u8=>7,37u8=>7,38u8=>7,39u8=>7,40u8=>7,41u8=>7,42u8=>7,43u8=>7,44u8=>7,45u8=>7,46u8=>7,47u8=>7,48u8=>7,49u8=>7,50u8=>7,51u8=>7,52u8=>7,53u8=>7,54u8=>7,55u8=>7,56u8=>7,57u8=>7,58u8=>7,59u8=>7,60u8=>7,61u8=>7,62u8=>7,63u8=>7,64u8=>7,65u8=>7,66u8=>7,67u8=>7,68u8=>7,69u8=>7,70u8=>7,71u8=>7,72u8=>7,73u8=>7,74u8=>7,75u8=>7,76u8=>7,77u8=>7,78u8=>7,79u8=>7,80u8=>7,81u8=>7,82u8=>7,83u8=>7,84u8=>7,85u8=>7,86u8=>7,87u8=>7,88u8=>7,89u8=>7,90u8=>7,91u8=>7,92u8=>7,93u8=>7,94u8=>7,95u8=>7,96u8=>7,97u8=>7,98u8=>7,99u8=>7,100u8=>7,101u8=>7,102u8=>7,103u8=>7,104u8=>7,105u8=>7,106u8=>7,107u8=>7,108u8=>7,109u8=>7,110u8=>7,111u8=>7,112u8=>7,113u8=>7,114u8=>7,115u8=>7,116u8=>7,117u8=>7,118u8=>7,119u8=>7,120u8=>7,121u8=>7,122u8=>7,123u8=>7,124u8=>7,125u8=>7,126u8=>7,127u8=>7,128u8=>7,129u8=>7,130u8=>7,131u8=>7,132u8=>7,133u8=>7,134u8=>7,135u8=>7,136u8=>7,137u8=>7,138u8=>7,139u8=>7,140u8=>7,141u8=>7,142u8=>7,143u8=>7,144u8=>7,145u8=>7,146u8=>7,147u8=>7,148u8=>7,149u8=>7,150u8=>7,151u8=>7,152u8=>7,153u8=>7,154u8=>7,155u8=>7,156u8=>7,157u8=>7,158u8=>7,159u8=>7,160u8=>7,161u8=>7,162u8=>7,163u8=>7,164u8=>7,165u8=>7,166u8=>7,167u8=>7,168u8=>7,169u8=>7,170u8=>7,171u8=>7,172u8=>7,173u8=>7,174u8=>7,175u8=>7,176u8=>7,177u8=>7,178u8=>7,179u8=>7,180u8=>7,181u8=>7,182u8=>7,183u8=>7,184u8=>7,185u8=>7,186u8=>7,187u8=>7,188u8=>7,189u8=>7,190u8=>7,191u8=>7,192u8=>7,193u8=>7,194u8=>7,195u8=>7,196u8=>7,197u8=>7,198u8=>7,199u8=>7,200u8=>7,201u8=>7,202u8=>7,203u8=>7,204u8=>7,205u8=>7,206u8=>7,207u8=>7,208u8=>7,209u8=>7,210u8=>7,211u8=>7,212u8=>7,213u8=>7,214u8=>7,215u8=>7,216u8=>7,217u8=>7,218u8=>7,219u8=>7,220u8=>7,221u8=>7,222u8=>7,223u8=>7,224u8=>7,225u8=>7,226u8=>7,227u8=>7,228u8=>7,229u8=>7,230u8=>7,231u8=>7,232u8=>7,233u8=>7,234u8=>7,235u8=>7,236u8=>7,237u8=>7,238u8=>7,239u8=>7,240u8=>7,241u8=>7,242u8=>7,243u8=>7,244u8=>7,245u8=>7,246u8=>7,247u8=>7,248u8=>7,249u8=>7,250u8=>7,251u8=>7,252u8=>7,253u8=>7,254u8=>7,255u8=>7}} fn run()->i32{classify(255u8)}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "struct S{a:i8,b:u64,c:i16} enum E{V(i8,u64)} fn relay(x:S)->S{x} fn run()->i32{let s=relay(S{a:-128i8,b:18446744073709551615u64,c:-32768i16});let a=[s.c;2];let r=&s;match E::V(r.a,r.b){E::V(x,y)=>if x==-128i8 && y==18446744073709551615u64 && a[1]==-32768i16{7}else{0}}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "mixed_integer_scalar_leaf_transport",
    "source": "struct S{a:i8,b:u64,c:i16} enum E{V(i8,u64)} fn relay(x:S)->S{x} fn run()->i32{let s=relay(S{a:-128i8,b:18446744073709551615u64,c:-32768i16});let a=[s.c;2];let r=&s;match E::V(r.a,r.b){E::V(x,y)=>if x==-128i8 && y==18446744073709551615u64 && a[1]==-32768i16{7}else{0}}}"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:i8,y:i8)->i8{x+y}\nfn main(){println!(\"before\");let _=op(127i8,1i8);println!(\"after\");}\n",
    "group": "runtime_error",
    "name": "i8_add_overflow",
    "source": "fn op(x:i8,y:i8)->i8{x+y}",
    "stock_panic_reason": "add with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:u8,y:u8)->u8{x-y}\nfn main(){println!(\"before\");let _=op(0u8,1u8);println!(\"after\");}\n",
    "group": "runtime_error",
    "name": "u8_subtract_overflow",
    "source": "fn op(x:u8,y:u8)->u8{x-y}",
    "stock_panic_reason": "subtract with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:i16,y:i16)->i16{x*y}\nfn main(){println!(\"before\");let _=op(300i16,300i16);println!(\"after\");}\n",
    "group": "runtime_error",
    "name": "i16_multiply_overflow",
    "source": "fn op(x:i16,y:i16)->i16{x*y}",
    "stock_panic_reason": "multiply with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:i64,y:i64)->i64{x+y}\nfn main(){println!(\"before\");let _=op(9223372036854775807i64,1i64);println!(\"after\");}\n",
    "group": "runtime_error",
    "name": "i64_add_overflow",
    "source": "fn op(x:i64,y:i64)->i64{x+y}",
    "stock_panic_reason": "add with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:u64,y:u64)->u64{x+y}\nfn main(){println!(\"before\");let _=op(18446744073709551615u64,1u64);println!(\"after\");}\n",
    "group": "runtime_error",
    "name": "u64_add_overflow",
    "source": "fn op(x:u64,y:u64)->u64{x+y}",
    "stock_panic_reason": "add with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:i32,y:i32)->i32{-x}\nfn main(){println!(\"before\");let _=op(-2147483648i32,0i32);println!(\"after\");}\n",
    "group": "runtime_error",
    "name": "i32_negate_minimum",
    "source": "fn op(x:i32,y:i32)->i32{-x}",
    "stock_panic_reason": "negate with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:u16,y:u16)->u16{x/y}\nfn main(){println!(\"before\");let _=op(7u16,0u16);println!(\"after\");}\n",
    "group": "runtime_error",
    "name": "u16_divide_zero",
    "source": "fn op(x:u16,y:u16)->u16{x/y}",
    "stock_panic_reason": "divide by zero"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:i8,y:i8)->i8{x/y}\nfn main(){println!(\"before\");let _=op(-128i8,-1i8);println!(\"after\");}\n",
    "group": "runtime_error",
    "name": "i8_minimum_divide_negative_one",
    "source": "fn op(x:i8,y:i8)->i8{x/y}",
    "stock_panic_reason": "divide with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:i64,y:i64)->i64{x%y}\nfn main(){println!(\"before\");let _=op(-9223372036854775808i64,-1i64);println!(\"after\");}\n",
    "group": "runtime_error",
    "name": "i64_minimum_remainder_negative_one",
    "source": "fn op(x:i64,y:i64)->i64{x%y}",
    "stock_panic_reason": "remainder with overflow"
  },
  {
    "expected": "overflowing_literals",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x:i8=128i8;7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "positive_signed_literal_out_of_range",
    "source": "fn run()->i32{let x:i8=128i8;7}",
    "stock_error_contains": "literal out of range"
  },
  {
    "expected": "overflowing_literals",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x:u8=256u8;7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "unsigned_literal_out_of_range",
    "source": "fn run()->i32{let x:u8=256u8;7}",
    "stock_error_contains": "literal out of range"
  },
  {
    "expected": "overflowing_literals",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x:i8=-129i8;7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "signed_literal_too_negative",
    "source": "fn run()->i32{let x:i8=-129i8;7}",
    "stock_error_contains": "literal out of range"
  },
  {
    "expected": "E0600",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x:u8=-1u8;7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "unsigned_negation_invalid",
    "source": "fn run()->i32{let x:u8=-1u8;7}"
  },
  {
    "expected": "E0308",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let a:i64=1isize;let b:usize=1u64;7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "equal_width_primitive_identity_mismatch",
    "source": "fn run()->i32{let a:i64=1isize;let b:usize=1u64;7}"
  },
  {
    "expected": "arithmetic_overflow",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x:i8=-{128i8};7}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "block_is_not_grouped_minimum",
    "source": "fn run()->i32{let x:i8=-{128i8};7}",
    "stock_error_contains": "overflow"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn id(x:i128)->i128{x} fn run()->i32{let x=id(-170141183460469231731687303715884105728i128);let y:u128=340282366920938463463374607431768211455u128;if x<0i128 && y>0u128{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "integer_128_types",
    "source": "fn id(x:i128)->i128{x} fn run()->i32{let x=id(-170141183460469231731687303715884105728i128);let y:u128=340282366920938463463374607431768211455u128;if x<0i128 && y>0u128{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "integer types oracle ok\n",
    "full_source": "fn run()->i32{let x:i8=-1i8;let y=x as u64;if y==18446744073709551615u64{7}else{0}}\nfn main() { if run() == 7 { println!(\"integer types oracle ok\"); } }\n",
    "group": "positive",
    "name": "integer_cast_syntax",
    "source": "fn run()->i32{let x:i8=-1i8;let y=x as u64;if y==18446744073709551615u64{7}else{0}}"
  }
]
''')
TRAPS = {f["name"]: f for f in FIXTURES if f["group"] == "runtime_error"}
assert len(TRAPS) == 9
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
    return run_case(case, *args)


def stock_case(case, *args):
    if case.kind == "accepted_trap101":
        return stock_trap_case(case, *args)
    return ordinary_stock_case(case, *args)


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
            prefix="jr-integer-types-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "integer-types-v1",
                "case_count": len(cases),
                "case_counts": {kind: sum(c.kind == kind for c in cases)
                                for kind in ("valid", "invalid", "boundary", "accepted_trap101")},
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
            raise SystemExit(f"{totals['failure']} integer-types discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
