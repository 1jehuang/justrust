#!/usr/bin/env python3
"""Bounded integer-casts differential corpus, not full Rust compatibility.

Exact frozen stock-classified source bytes/stdout mirror bear's integer-cast acceptance
corpus. Only casts among ten <=64-bit integer types are claimed.
Noninteger conversions and wider integer types
remain explicit boundaries. Valid runs must exit zero, exact stdout, empty stderr.
Reuse module_correctness Case/run_case/process, deadline-bound file-backed
capture and supplied-native workers 1/8 check/build. Never build a binary here.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "942778196cb187edd61937b08c38f1c30871af1dff2145dfcbc6857323a782b3"

# Authoritative frozen bear oracle. full_source has no added wrapper.
FIXTURES = json.loads(r'''
[
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn check0(x:i8)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:i8)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:i8)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn check3(x:i8)->i32{if (x as i8) != -128i8 {return 0;}if (x as i16) != -128i16 {return 0;}if (x as i32) != -128i32 {return 0;}if (x as i64) != -128i64 {return 0;}if (x as isize) != -128isize {return 0;}if (x as u8) != 128u8 {return 0;}if (x as u16) != 65408u16 {return 0;}if (x as u32) != 4294967168u32 {return 0;}if (x as u64) != 18446744073709551488u64 {return 0;}if (x as usize) != 18446744073709551488usize {return 0;}7}\nfn check4(x:i8)->i32{if (x as i8) != 127i8 {return 0;}if (x as i16) != 127i16 {return 0;}if (x as i32) != 127i32 {return 0;}if (x as i64) != 127i64 {return 0;}if (x as isize) != 127isize {return 0;}if (x as u8) != 127u8 {return 0;}if (x as u16) != 127u16 {return 0;}if (x as u32) != 127u32 {return 0;}if (x as u64) != 127u64 {return 0;}if (x as usize) != 127usize {return 0;}7}\nfn run()->i32{if check0(0i8)!=7{return 0;}if check1(1i8)!=7{return 0;}if check2(-1i8)!=7{return 0;}if check3(-128i8)!=7{return 0;}if check4(127i8)!=7{return 0;}7}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "matrix_from_i8",
    "source": "fn check0(x:i8)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:i8)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:i8)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn check3(x:i8)->i32{if (x as i8) != -128i8 {return 0;}if (x as i16) != -128i16 {return 0;}if (x as i32) != -128i32 {return 0;}if (x as i64) != -128i64 {return 0;}if (x as isize) != -128isize {return 0;}if (x as u8) != 128u8 {return 0;}if (x as u16) != 65408u16 {return 0;}if (x as u32) != 4294967168u32 {return 0;}if (x as u64) != 18446744073709551488u64 {return 0;}if (x as usize) != 18446744073709551488usize {return 0;}7}\nfn check4(x:i8)->i32{if (x as i8) != 127i8 {return 0;}if (x as i16) != 127i16 {return 0;}if (x as i32) != 127i32 {return 0;}if (x as i64) != 127i64 {return 0;}if (x as isize) != 127isize {return 0;}if (x as u8) != 127u8 {return 0;}if (x as u16) != 127u16 {return 0;}if (x as u32) != 127u32 {return 0;}if (x as u64) != 127u64 {return 0;}if (x as usize) != 127usize {return 0;}7}\nfn run()->i32{if check0(0i8)!=7{return 0;}if check1(1i8)!=7{return 0;}if check2(-1i8)!=7{return 0;}if check3(-128i8)!=7{return 0;}if check4(127i8)!=7{return 0;}7}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn check0(x:i16)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:i16)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:i16)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn check3(x:i16)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != -32768i16 {return 0;}if (x as i32) != -32768i32 {return 0;}if (x as i64) != -32768i64 {return 0;}if (x as isize) != -32768isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 32768u16 {return 0;}if (x as u32) != 4294934528u32 {return 0;}if (x as u64) != 18446744073709518848u64 {return 0;}if (x as usize) != 18446744073709518848usize {return 0;}7}\nfn check4(x:i16)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != 32767i16 {return 0;}if (x as i32) != 32767i32 {return 0;}if (x as i64) != 32767i64 {return 0;}if (x as isize) != 32767isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 32767u16 {return 0;}if (x as u32) != 32767u32 {return 0;}if (x as u64) != 32767u64 {return 0;}if (x as usize) != 32767usize {return 0;}7}\nfn run()->i32{if check0(0i16)!=7{return 0;}if check1(1i16)!=7{return 0;}if check2(-1i16)!=7{return 0;}if check3(-32768i16)!=7{return 0;}if check4(32767i16)!=7{return 0;}7}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "matrix_from_i16",
    "source": "fn check0(x:i16)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:i16)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:i16)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn check3(x:i16)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != -32768i16 {return 0;}if (x as i32) != -32768i32 {return 0;}if (x as i64) != -32768i64 {return 0;}if (x as isize) != -32768isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 32768u16 {return 0;}if (x as u32) != 4294934528u32 {return 0;}if (x as u64) != 18446744073709518848u64 {return 0;}if (x as usize) != 18446744073709518848usize {return 0;}7}\nfn check4(x:i16)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != 32767i16 {return 0;}if (x as i32) != 32767i32 {return 0;}if (x as i64) != 32767i64 {return 0;}if (x as isize) != 32767isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 32767u16 {return 0;}if (x as u32) != 32767u32 {return 0;}if (x as u64) != 32767u64 {return 0;}if (x as usize) != 32767usize {return 0;}7}\nfn run()->i32{if check0(0i16)!=7{return 0;}if check1(1i16)!=7{return 0;}if check2(-1i16)!=7{return 0;}if check3(-32768i16)!=7{return 0;}if check4(32767i16)!=7{return 0;}7}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn check0(x:i32)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:i32)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:i32)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn check3(x:i32)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != -2147483648i32 {return 0;}if (x as i64) != -2147483648i64 {return 0;}if (x as isize) != -2147483648isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 2147483648u32 {return 0;}if (x as u64) != 18446744071562067968u64 {return 0;}if (x as usize) != 18446744071562067968usize {return 0;}7}\nfn check4(x:i32)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != 2147483647i32 {return 0;}if (x as i64) != 2147483647i64 {return 0;}if (x as isize) != 2147483647isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 2147483647u32 {return 0;}if (x as u64) != 2147483647u64 {return 0;}if (x as usize) != 2147483647usize {return 0;}7}\nfn run()->i32{if check0(0i32)!=7{return 0;}if check1(1i32)!=7{return 0;}if check2(-1i32)!=7{return 0;}if check3(-2147483648i32)!=7{return 0;}if check4(2147483647i32)!=7{return 0;}7}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "matrix_from_i32",
    "source": "fn check0(x:i32)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:i32)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:i32)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn check3(x:i32)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != -2147483648i32 {return 0;}if (x as i64) != -2147483648i64 {return 0;}if (x as isize) != -2147483648isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 2147483648u32 {return 0;}if (x as u64) != 18446744071562067968u64 {return 0;}if (x as usize) != 18446744071562067968usize {return 0;}7}\nfn check4(x:i32)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != 2147483647i32 {return 0;}if (x as i64) != 2147483647i64 {return 0;}if (x as isize) != 2147483647isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 2147483647u32 {return 0;}if (x as u64) != 2147483647u64 {return 0;}if (x as usize) != 2147483647usize {return 0;}7}\nfn run()->i32{if check0(0i32)!=7{return 0;}if check1(1i32)!=7{return 0;}if check2(-1i32)!=7{return 0;}if check3(-2147483648i32)!=7{return 0;}if check4(2147483647i32)!=7{return 0;}7}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn check0(x:i64)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:i64)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:i64)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn check3(x:i64)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != -9223372036854775808i64 {return 0;}if (x as isize) != -9223372036854775808isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 9223372036854775808u64 {return 0;}if (x as usize) != 9223372036854775808usize {return 0;}7}\nfn check4(x:i64)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != 9223372036854775807i64 {return 0;}if (x as isize) != 9223372036854775807isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 9223372036854775807u64 {return 0;}if (x as usize) != 9223372036854775807usize {return 0;}7}\nfn run()->i32{if check0(0i64)!=7{return 0;}if check1(1i64)!=7{return 0;}if check2(-1i64)!=7{return 0;}if check3(-9223372036854775808i64)!=7{return 0;}if check4(9223372036854775807i64)!=7{return 0;}7}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "matrix_from_i64",
    "source": "fn check0(x:i64)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:i64)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:i64)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn check3(x:i64)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != -9223372036854775808i64 {return 0;}if (x as isize) != -9223372036854775808isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 9223372036854775808u64 {return 0;}if (x as usize) != 9223372036854775808usize {return 0;}7}\nfn check4(x:i64)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != 9223372036854775807i64 {return 0;}if (x as isize) != 9223372036854775807isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 9223372036854775807u64 {return 0;}if (x as usize) != 9223372036854775807usize {return 0;}7}\nfn run()->i32{if check0(0i64)!=7{return 0;}if check1(1i64)!=7{return 0;}if check2(-1i64)!=7{return 0;}if check3(-9223372036854775808i64)!=7{return 0;}if check4(9223372036854775807i64)!=7{return 0;}7}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn check0(x:isize)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:isize)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:isize)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn check3(x:isize)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != -9223372036854775808i64 {return 0;}if (x as isize) != -9223372036854775808isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 9223372036854775808u64 {return 0;}if (x as usize) != 9223372036854775808usize {return 0;}7}\nfn check4(x:isize)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != 9223372036854775807i64 {return 0;}if (x as isize) != 9223372036854775807isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 9223372036854775807u64 {return 0;}if (x as usize) != 9223372036854775807usize {return 0;}7}\nfn run()->i32{if check0(0isize)!=7{return 0;}if check1(1isize)!=7{return 0;}if check2(-1isize)!=7{return 0;}if check3(-9223372036854775808isize)!=7{return 0;}if check4(9223372036854775807isize)!=7{return 0;}7}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "matrix_from_isize",
    "source": "fn check0(x:isize)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:isize)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:isize)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn check3(x:isize)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != -9223372036854775808i64 {return 0;}if (x as isize) != -9223372036854775808isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 9223372036854775808u64 {return 0;}if (x as usize) != 9223372036854775808usize {return 0;}7}\nfn check4(x:isize)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != 9223372036854775807i64 {return 0;}if (x as isize) != 9223372036854775807isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 9223372036854775807u64 {return 0;}if (x as usize) != 9223372036854775807usize {return 0;}7}\nfn run()->i32{if check0(0isize)!=7{return 0;}if check1(1isize)!=7{return 0;}if check2(-1isize)!=7{return 0;}if check3(-9223372036854775808isize)!=7{return 0;}if check4(9223372036854775807isize)!=7{return 0;}7}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn check0(x:u8)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:u8)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:u8)->i32{if (x as i8) != -128i8 {return 0;}if (x as i16) != 128i16 {return 0;}if (x as i32) != 128i32 {return 0;}if (x as i64) != 128i64 {return 0;}if (x as isize) != 128isize {return 0;}if (x as u8) != 128u8 {return 0;}if (x as u16) != 128u16 {return 0;}if (x as u32) != 128u32 {return 0;}if (x as u64) != 128u64 {return 0;}if (x as usize) != 128usize {return 0;}7}\nfn check3(x:u8)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != 255i16 {return 0;}if (x as i32) != 255i32 {return 0;}if (x as i64) != 255i64 {return 0;}if (x as isize) != 255isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 255u16 {return 0;}if (x as u32) != 255u32 {return 0;}if (x as u64) != 255u64 {return 0;}if (x as usize) != 255usize {return 0;}7}\nfn run()->i32{if check0(0u8)!=7{return 0;}if check1(1u8)!=7{return 0;}if check2(128u8)!=7{return 0;}if check3(255u8)!=7{return 0;}7}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "matrix_from_u8",
    "source": "fn check0(x:u8)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:u8)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:u8)->i32{if (x as i8) != -128i8 {return 0;}if (x as i16) != 128i16 {return 0;}if (x as i32) != 128i32 {return 0;}if (x as i64) != 128i64 {return 0;}if (x as isize) != 128isize {return 0;}if (x as u8) != 128u8 {return 0;}if (x as u16) != 128u16 {return 0;}if (x as u32) != 128u32 {return 0;}if (x as u64) != 128u64 {return 0;}if (x as usize) != 128usize {return 0;}7}\nfn check3(x:u8)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != 255i16 {return 0;}if (x as i32) != 255i32 {return 0;}if (x as i64) != 255i64 {return 0;}if (x as isize) != 255isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 255u16 {return 0;}if (x as u32) != 255u32 {return 0;}if (x as u64) != 255u64 {return 0;}if (x as usize) != 255usize {return 0;}7}\nfn run()->i32{if check0(0u8)!=7{return 0;}if check1(1u8)!=7{return 0;}if check2(128u8)!=7{return 0;}if check3(255u8)!=7{return 0;}7}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn check0(x:u16)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:u16)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:u16)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != -32768i16 {return 0;}if (x as i32) != 32768i32 {return 0;}if (x as i64) != 32768i64 {return 0;}if (x as isize) != 32768isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 32768u16 {return 0;}if (x as u32) != 32768u32 {return 0;}if (x as u64) != 32768u64 {return 0;}if (x as usize) != 32768usize {return 0;}7}\nfn check3(x:u16)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != 65535i32 {return 0;}if (x as i64) != 65535i64 {return 0;}if (x as isize) != 65535isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 65535u32 {return 0;}if (x as u64) != 65535u64 {return 0;}if (x as usize) != 65535usize {return 0;}7}\nfn run()->i32{if check0(0u16)!=7{return 0;}if check1(1u16)!=7{return 0;}if check2(32768u16)!=7{return 0;}if check3(65535u16)!=7{return 0;}7}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "matrix_from_u16",
    "source": "fn check0(x:u16)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:u16)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:u16)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != -32768i16 {return 0;}if (x as i32) != 32768i32 {return 0;}if (x as i64) != 32768i64 {return 0;}if (x as isize) != 32768isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 32768u16 {return 0;}if (x as u32) != 32768u32 {return 0;}if (x as u64) != 32768u64 {return 0;}if (x as usize) != 32768usize {return 0;}7}\nfn check3(x:u16)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != 65535i32 {return 0;}if (x as i64) != 65535i64 {return 0;}if (x as isize) != 65535isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 65535u32 {return 0;}if (x as u64) != 65535u64 {return 0;}if (x as usize) != 65535usize {return 0;}7}\nfn run()->i32{if check0(0u16)!=7{return 0;}if check1(1u16)!=7{return 0;}if check2(32768u16)!=7{return 0;}if check3(65535u16)!=7{return 0;}7}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn check0(x:u32)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:u32)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:u32)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != -2147483648i32 {return 0;}if (x as i64) != 2147483648i64 {return 0;}if (x as isize) != 2147483648isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 2147483648u32 {return 0;}if (x as u64) != 2147483648u64 {return 0;}if (x as usize) != 2147483648usize {return 0;}7}\nfn check3(x:u32)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != 4294967295i64 {return 0;}if (x as isize) != 4294967295isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 4294967295u64 {return 0;}if (x as usize) != 4294967295usize {return 0;}7}\nfn run()->i32{if check0(0u32)!=7{return 0;}if check1(1u32)!=7{return 0;}if check2(2147483648u32)!=7{return 0;}if check3(4294967295u32)!=7{return 0;}7}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "matrix_from_u32",
    "source": "fn check0(x:u32)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:u32)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:u32)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != -2147483648i32 {return 0;}if (x as i64) != 2147483648i64 {return 0;}if (x as isize) != 2147483648isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 2147483648u32 {return 0;}if (x as u64) != 2147483648u64 {return 0;}if (x as usize) != 2147483648usize {return 0;}7}\nfn check3(x:u32)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != 4294967295i64 {return 0;}if (x as isize) != 4294967295isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 4294967295u64 {return 0;}if (x as usize) != 4294967295usize {return 0;}7}\nfn run()->i32{if check0(0u32)!=7{return 0;}if check1(1u32)!=7{return 0;}if check2(2147483648u32)!=7{return 0;}if check3(4294967295u32)!=7{return 0;}7}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn check0(x:u64)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:u64)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:u64)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != -9223372036854775808i64 {return 0;}if (x as isize) != -9223372036854775808isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 9223372036854775808u64 {return 0;}if (x as usize) != 9223372036854775808usize {return 0;}7}\nfn check3(x:u64)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn run()->i32{if check0(0u64)!=7{return 0;}if check1(1u64)!=7{return 0;}if check2(9223372036854775808u64)!=7{return 0;}if check3(18446744073709551615u64)!=7{return 0;}7}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "matrix_from_u64",
    "source": "fn check0(x:u64)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:u64)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:u64)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != -9223372036854775808i64 {return 0;}if (x as isize) != -9223372036854775808isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 9223372036854775808u64 {return 0;}if (x as usize) != 9223372036854775808usize {return 0;}7}\nfn check3(x:u64)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn run()->i32{if check0(0u64)!=7{return 0;}if check1(1u64)!=7{return 0;}if check2(9223372036854775808u64)!=7{return 0;}if check3(18446744073709551615u64)!=7{return 0;}7}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn check0(x:usize)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:usize)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:usize)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != -9223372036854775808i64 {return 0;}if (x as isize) != -9223372036854775808isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 9223372036854775808u64 {return 0;}if (x as usize) != 9223372036854775808usize {return 0;}7}\nfn check3(x:usize)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn run()->i32{if check0(0usize)!=7{return 0;}if check1(1usize)!=7{return 0;}if check2(9223372036854775808usize)!=7{return 0;}if check3(18446744073709551615usize)!=7{return 0;}7}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "matrix_from_usize",
    "source": "fn check0(x:usize)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}\nfn check1(x:usize)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}\nfn check2(x:usize)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != -9223372036854775808i64 {return 0;}if (x as isize) != -9223372036854775808isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 9223372036854775808u64 {return 0;}if (x as usize) != 9223372036854775808usize {return 0;}7}\nfn check3(x:usize)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}\nfn run()->i32{if check0(0usize)!=7{return 0;}if check1(1usize)!=7{return 0;}if check2(9223372036854775808usize)!=7{return 0;}if check3(18446744073709551615usize)!=7{return 0;}7}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x=300i32 as u8;if x==44u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "suffixed_narrow",
    "source": "fn run()->i32{let x=300i32 as u8;if x==44u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x=(200+100) as u8;if x==44u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "binary_drops_hint",
    "source": "fn run()->i32{let x=(200+100) as u8;if x==44u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x={let y=1;127+y-1;255} as u8;if x==255u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "block_tail_hint",
    "source": "fn run()->i32{let x={let y=1;127+y-1;255} as u8;if x==255u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x={200+100} as u8;if x==44u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "block_binary_tail",
    "source": "fn run()->i32{let x={200+100} as u8;if x==44u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x=(if true{300}else{1}) as u8;if x==44u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "if_drops_hint",
    "source": "fn run()->i32{let x=(if true{300}else{1}) as u8;if x==44u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x=(match true{true=>300,false=>1}) as u8;if x==44u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "match_drops_hint",
    "source": "fn run()->i32{let x=(match true{true=>300,false=>1}) as u8;if x==44u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x=(loop{break 300;}) as u8;if x==44u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "loop_drops_hint",
    "source": "fn run()->i32{let x=(loop{break 300;}) as u8;if x==44u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x=(!0) as u8;if x==255u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "not_forwards_hint",
    "source": "fn run()->i32{let x=(!0) as u8;if x==255u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x=(-128) as i8;if x == -128i8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "negative_signed_hint",
    "source": "fn run()->i32{let x=(-128) as i8;if x == -128i8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x=(255 as u8) as i16;if x==255i16{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "nested_cast_own_hint",
    "source": "fn run()->i32{let x=(255 as u8) as i16;if x==255i16{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x=511i32 as u8 as i16;if x==255i16{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "left_associative_cast_chain",
    "source": "fn run()->i32{let x=511i32 as u8 as i16;if x==255i16{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x=-2i16 as u8 * 1u8;if x==254u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "unary_cast_multiply_precedence",
    "source": "fn run()->i32{let x=-2i16 as u8 * 1u8;if x==254u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "struct S{x:i16}fn run()->i32{let s=S{x:-1i16};let a=[300i32];let v=65535u16;if s.x as u32==4294967295u32 && a[0] as u8==44u8 && v as i16 == -1i16{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "concrete_local_field_index",
    "source": "struct S{x:i16}fn run()->i32{let s=S{x:-1i16};let a=[300i32];let v=65535u16;if s.x as u32==4294967295u32 && a[0] as u8==44u8 && v as i16 == -1i16{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "struct S{x:i16}fn run()->i32{let mut s=S{x:-1i16};let r=&s;let n=r.x as u64;s.x=7i16;if n==18446744073709551615u64 && s.x==7i16{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "reference_projection_last_use",
    "source": "struct S{x:i16}fn run()->i32{let mut s=S{x:-1i16};let r=&s;let n=r.x as u64;s.x=7i16;if n==18446744073709551615u64 && s.x==7i16{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let mut n=0;let x={n=n+1;300i32} as u8;if n==1 && x==44u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "once_evaluation",
    "source": "fn run()->i32{let mut n=0;let x={n=n+1;300i32} as u8;if n==1 && x==44u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn value()->i32{300}fn run()->i32{let x=value() as u8;if x==44u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "positive",
    "name": "concrete_call",
    "source": "fn value()->i32{300}fn run()->i32{let x=value() as u8;if x==44u8{7}else{0}}"
  },
  {
    "expected": "overflowing_literals",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=300 as u8;7}\nfn main(){run();}",
    "group": "negative",
    "name": "unsuffixed_source_out_of_range",
    "source": "fn run()->i32{let x=300 as u8;7}"
  },
  {
    "expected": "E0600",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=(-1) as u8;7}\nfn main(){run();}",
    "group": "negative",
    "name": "unsigned_negative_hint",
    "source": "fn run()->i32{let x=(-1) as u8;7}"
  },
  {
    "expected": "overflowing_literals",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=256u8 as u16;7}\nfn main(){run();}",
    "group": "negative",
    "name": "suffixed_source_out_of_range",
    "source": "fn run()->i32{let x=256u8 as u16;7}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{if true as u8==1u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "legalunsupported",
    "name": "bool_to_integer",
    "source": "fn run()->i32{if true as u8==1u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x=7;let p=&x as *const i32;7}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "legalunsupported",
    "name": "reference_to_pointer",
    "source": "fn run()->i32{let x=7;let p=&x as *const i32;7}"
  },
  {
    "expected": 7,
    "expected_stdout": "cast oracle ok\n",
    "full_source": "fn run()->i32{let x=300u128 as u8;if x==44u8{7}else{0}}\nfn main(){if run()==7{println!(\"cast oracle ok\");}}",
    "group": "legalunsupported",
    "name": "integer128_cast",
    "source": "fn run()->i32{let x=300u128 as u8;if x==44u8{7}else{0}}"
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
    assert len(cases) == 32 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-integer-casts-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "integer-casts-v1",
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
            raise SystemExit(f"{totals['failure']} integer-casts discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
