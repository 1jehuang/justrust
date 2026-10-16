#!/usr/bin/env python3
"""Bounded <=64-bit shift oracle, not full Rust compatibility.

Exact stock-classified sources/stdout mirror tests/shifts.rs. Ordinary cases use
unchanged shared Case/run_case/process infrastructure and strict output parity.
Only declared runtime_error fixtures allow native versus stock panic stderr text
to differ: both runs must finish exactly101 with exact stdout, stock panic marker
and reason verified. Signals, timeout, truncation and compiler crashes never pass.
Native binaries are supplied, never built by this harness. Two declared static
invalids accept stock metadata but must reject stock BUILD and native BUILD/CHECK.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time

from module_correctness import Case, accepted, completed, process, rejected, run_case


ACCEPTANCE_CORPUS_SHA256 = "10c83f1d5cb347684655cae77ee175f74d16444c8fa0e00b2f4c9bb405a403de"
TRAP_POLICY = ("declared runtime_error only: compile/check0, executable required, "
               "completed runtime101 and exact stdout; stock stderr contains panicked at "
               "and fixture reason; native stderr captured but may differ or be empty; "
               "signals/timeouts/truncation/compiler crashes fail")

# Frozen bear oracle. full_source is authoritative, without additional wrappers.
FIXTURES = json.loads(r'''
[
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn left(x:i8,n:u64)->i8{x<<n}fn right(x:i8,n:i16)->i8{x>>n}fn run()->i32{let a=left(1i8,7u64);let b=left(-1i8,1u64);let z=right(-128i8,7i16);if a==-128i8 && b==-2i8 && z==-1i8{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "shift_i8_edges",
    "source": "fn left(x:i8,n:u64)->i8{x<<n}fn right(x:i8,n:i16)->i8{x>>n}fn run()->i32{let a=left(1i8,7u64);let b=left(-1i8,1u64);let z=right(-128i8,7i16);if a==-128i8 && b==-2i8 && z==-1i8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn left(x:i16,n:u64)->i16{x<<n}fn right(x:i16,n:i16)->i16{x>>n}fn run()->i32{let a=left(1i16,15u64);let b=left(-1i16,1u64);let z=right(-32768i16,15i16);if a==-32768i16 && b==-2i16 && z==-1i16{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "shift_i16_edges",
    "source": "fn left(x:i16,n:u64)->i16{x<<n}fn right(x:i16,n:i16)->i16{x>>n}fn run()->i32{let a=left(1i16,15u64);let b=left(-1i16,1u64);let z=right(-32768i16,15i16);if a==-32768i16 && b==-2i16 && z==-1i16{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn left(x:i32,n:u64)->i32{x<<n}fn right(x:i32,n:i16)->i32{x>>n}fn run()->i32{let a=left(1i32,31u64);let b=left(-1i32,1u64);let z=right(-2147483648i32,31i16);if a==-2147483648i32 && b==-2i32 && z==-1i32{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "shift_i32_edges",
    "source": "fn left(x:i32,n:u64)->i32{x<<n}fn right(x:i32,n:i16)->i32{x>>n}fn run()->i32{let a=left(1i32,31u64);let b=left(-1i32,1u64);let z=right(-2147483648i32,31i16);if a==-2147483648i32 && b==-2i32 && z==-1i32{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn left(x:i64,n:u64)->i64{x<<n}fn right(x:i64,n:i16)->i64{x>>n}fn run()->i32{let a=left(1i64,63u64);let b=left(-1i64,1u64);let z=right(-9223372036854775808i64,63i16);if a==-9223372036854775808i64 && b==-2i64 && z==-1i64{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "shift_i64_edges",
    "source": "fn left(x:i64,n:u64)->i64{x<<n}fn right(x:i64,n:i16)->i64{x>>n}fn run()->i32{let a=left(1i64,63u64);let b=left(-1i64,1u64);let z=right(-9223372036854775808i64,63i16);if a==-9223372036854775808i64 && b==-2i64 && z==-1i64{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn left(x:isize,n:u64)->isize{x<<n}fn right(x:isize,n:i16)->isize{x>>n}fn run()->i32{let a=left(1isize,63u64);let b=left(-1isize,1u64);let z=right(-9223372036854775808isize,63i16);if a==-9223372036854775808isize && b==-2isize && z==-1isize{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "shift_isize_edges",
    "source": "fn left(x:isize,n:u64)->isize{x<<n}fn right(x:isize,n:i16)->isize{x>>n}fn run()->i32{let a=left(1isize,63u64);let b=left(-1isize,1u64);let z=right(-9223372036854775808isize,63i16);if a==-9223372036854775808isize && b==-2isize && z==-1isize{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn left(x:u8,n:u64)->u8{x<<n}fn right(x:u8,n:i16)->u8{x>>n}fn run()->i32{let a=left(1u8,7u64);let b=left(255u8,1u64);let z=right(128u8,7i16);if a==128u8 && b==254u8 && z==1u8{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "shift_u8_edges",
    "source": "fn left(x:u8,n:u64)->u8{x<<n}fn right(x:u8,n:i16)->u8{x>>n}fn run()->i32{let a=left(1u8,7u64);let b=left(255u8,1u64);let z=right(128u8,7i16);if a==128u8 && b==254u8 && z==1u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn left(x:u16,n:u64)->u16{x<<n}fn right(x:u16,n:i16)->u16{x>>n}fn run()->i32{let a=left(1u16,15u64);let b=left(65535u16,1u64);let z=right(32768u16,15i16);if a==32768u16 && b==65534u16 && z==1u16{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "shift_u16_edges",
    "source": "fn left(x:u16,n:u64)->u16{x<<n}fn right(x:u16,n:i16)->u16{x>>n}fn run()->i32{let a=left(1u16,15u64);let b=left(65535u16,1u64);let z=right(32768u16,15i16);if a==32768u16 && b==65534u16 && z==1u16{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn left(x:u32,n:u64)->u32{x<<n}fn right(x:u32,n:i16)->u32{x>>n}fn run()->i32{let a=left(1u32,31u64);let b=left(4294967295u32,1u64);let z=right(2147483648u32,31i16);if a==2147483648u32 && b==4294967294u32 && z==1u32{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "shift_u32_edges",
    "source": "fn left(x:u32,n:u64)->u32{x<<n}fn right(x:u32,n:i16)->u32{x>>n}fn run()->i32{let a=left(1u32,31u64);let b=left(4294967295u32,1u64);let z=right(2147483648u32,31i16);if a==2147483648u32 && b==4294967294u32 && z==1u32{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn left(x:u64,n:u64)->u64{x<<n}fn right(x:u64,n:i16)->u64{x>>n}fn run()->i32{let a=left(1u64,63u64);let b=left(18446744073709551615u64,1u64);let z=right(9223372036854775808u64,63i16);if a==9223372036854775808u64 && b==18446744073709551614u64 && z==1u64{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "shift_u64_edges",
    "source": "fn left(x:u64,n:u64)->u64{x<<n}fn right(x:u64,n:i16)->u64{x>>n}fn run()->i32{let a=left(1u64,63u64);let b=left(18446744073709551615u64,1u64);let z=right(9223372036854775808u64,63i16);if a==9223372036854775808u64 && b==18446744073709551614u64 && z==1u64{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn left(x:usize,n:u64)->usize{x<<n}fn right(x:usize,n:i16)->usize{x>>n}fn run()->i32{let a=left(1usize,63u64);let b=left(18446744073709551615usize,1u64);let z=right(9223372036854775808usize,63i16);if a==9223372036854775808usize && b==18446744073709551614usize && z==1usize{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "shift_usize_edges",
    "source": "fn left(x:usize,n:u64)->usize{x<<n}fn right(x:usize,n:i16)->usize{x>>n}fn run()->i32{let a=left(1usize,63u64);let b=left(18446744073709551615usize,1u64);let z=right(9223372036854775808usize,63i16);if a==9223372036854775808usize && b==18446744073709551614usize && z==1usize{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn lhs(p:&mut i32)->u16{*p=*p*10+1;3u16}fn rhs(p:&mut i32)->u8{*p=*p*10+2;2u8}fn run()->i32{let mut n=0;let x=lhs(&mut n)<<rhs(&mut n);if n==12 && x==12u16{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "once_left_to_right",
    "source": "fn lhs(p:&mut i32)->u16{*p=*p*10+1;3u16}fn rhs(p:&mut i32)->u8{*p=*p*10+2;2u8}fn run()->i32{let mut n=0;let x=lhs(&mut n)<<rhs(&mut n);if n==12 && x==12u16{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn run()->i32{let x=1u16<<1u8+2u8;let y=1u16|2u16<<2u8;let z=256u16 as u8>>1u8;if x==8u16 && y==9u16 && z==0u8{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "precedence_cast_arithmetic_bitwise",
    "source": "fn run()->i32{let x=1u16<<1u8+2u8;let y=1u16|2u16<<2u8;let z=256u16 as u8>>1u8;if x==8u16 && y==9u16 && z==0u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn run()->i32{let x=1u16<<3u8>>1i64;if x==4u16{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "left_associative_chain",
    "source": "fn run()->i32{let x=1u16<<3u8>>1i64;if x==4u16{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn run()->i32{let x:u8=1<<2u64;let y:i16=-8>>1u8;if x==4u8 && y == -4i16{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "left_context_rhs_independent",
    "source": "fn run()->i32{let x:u8=1<<2u64;let y:i16=-8>>1u8;if x==4u8 && y == -4i16{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "struct S{x:u64,y:i64}fn run()->i32{let s=S{x:9223372036854775808u64,y:-8i64};let r=&s;let a=[r.x>>63u8];let p=(a[0],r.y>>2usize);if p.0==1u64 && p.1 == -2i64{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "positive",
    "name": "reference_aggregate_transport",
    "source": "struct S{x:u64,y:i64}fn run()->i32{let s=S{x:9223372036854775808u64,y:-8i64};let r=&s;let a=[r.x>>63u8];let p=(a[0],r.y>>2usize);if p.0==1u64 && p.1 == -2i64{7}else{0}}"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:u8,n:i8)->u8{x<<n}\nfn main(){println!(\"before\");let x=op(1u8,-1i8);println!(\"after\");}",
    "group": "runtime_error",
    "name": "negative_left",
    "source": "fn op(x:u8,n:i8)->u8{x<<n}",
    "stock_panic_reason": "attempt to shift left with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:i64,n:i64)->i64{x>>n}\nfn main(){println!(\"before\");let x=op(1i64,-1i64);println!(\"after\");}",
    "group": "runtime_error",
    "name": "negative_right",
    "source": "fn op(x:i64,n:i64)->i64{x>>n}",
    "stock_panic_reason": "attempt to shift right with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:u8,n:u16)->u8{x<<n}\nfn main(){println!(\"before\");let x=op(1u8,8u16);println!(\"after\");}",
    "group": "runtime_error",
    "name": "exact_width_u8",
    "source": "fn op(x:u8,n:u16)->u8{x<<n}",
    "stock_panic_reason": "attempt to shift left with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:i64,n:u8)->i64{x>>n}\nfn main(){println!(\"before\");let x=op(1i64,64u8);println!(\"after\");}",
    "group": "runtime_error",
    "name": "exact_width_i64",
    "source": "fn op(x:i64,n:u8)->i64{x>>n}",
    "stock_panic_reason": "attempt to shift right with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:usize,n:usize)->usize{x<<n}\nfn main(){println!(\"before\");let x=op(1usize,64usize);println!(\"after\");}",
    "group": "runtime_error",
    "name": "exact_width_usize",
    "source": "fn op(x:usize,n:usize)->usize{x<<n}",
    "stock_panic_reason": "attempt to shift left with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:u8,n:u64)->u8{x<<n}\nfn main(){println!(\"before\");let x=op(1u8,18446744073709551615u64);println!(\"after\");}",
    "group": "runtime_error",
    "name": "huge_u64_left",
    "source": "fn op(x:u8,n:u64)->u8{x<<n}",
    "stock_panic_reason": "attempt to shift left with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:u64,n:u64)->u64{x>>n}\nfn main(){println!(\"before\");let x=op(1u64,18446744073709551615u64);println!(\"after\");}",
    "group": "runtime_error",
    "name": "huge_u64_right",
    "source": "fn op(x:u64,n:u64)->u64{x>>n}",
    "stock_panic_reason": "attempt to shift right with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:u16,n:i8)->u16{x>>n}\nfn main(){println!(\"before\");let x=op(1u16,-128i8);println!(\"after\");}",
    "group": "runtime_error",
    "name": "negative_minimum",
    "source": "fn op(x:u16,n:i8)->u16{x>>n}",
    "stock_panic_reason": "attempt to shift right with overflow"
  },
  {
    "expected": 101,
    "expected_stdout": "before\n",
    "full_source": "fn op(x:u32,n:u64)->u32{x<<n}\nfn main(){println!(\"before\");let x=op(1u32,4294967296u64);println!(\"after\");}",
    "group": "runtime_error",
    "name": "above_width_u32",
    "source": "fn op(x:u32,n:u64)->u32{x<<n}",
    "stock_panic_reason": "attempt to shift left with overflow"
  },
  {
    "expected": "E0369",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=true<<1u8;7}\nfn main(){run();}",
    "group": "negative",
    "name": "bool_left",
    "source": "fn run()->i32{let x=true<<1u8;7}"
  },
  {
    "expected": "E0277",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=1u8>>true;7}\nfn main(){run();}",
    "group": "negative",
    "name": "bool_count",
    "source": "fn run()->i32{let x=1u8>>true;7}"
  },
  {
    "expected": "arithmetic_overflow",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=1u8<<8u8;7}\nfn main(){run();}",
    "group": "negative",
    "name": "constant_width_overflow",
    "source": "fn run()->i32{let x=1u8<<8u8;7}",
    "stock_check_status": 0
  },
  {
    "expected": "arithmetic_overflow",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=1u16>>-1i8;7}\nfn main(){run();}",
    "group": "negative",
    "name": "constant_negative_count",
    "source": "fn run()->i32{let x=1u16>>-1i8;7}",
    "stock_check_status": 0
  },
  {
    "expected": "overflowing_literals",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=256u8<<0u8;7}\nfn main(){run();}",
    "group": "negative",
    "name": "source_literal_range",
    "source": "fn run()->i32{let x=256u8<<0u8;7}"
  },
  {
    "expected": "overflowing_literals",
    "expected_stdout": "",
    "full_source": "fn run()->i32{let x=1u8<<256u8;7}\nfn main(){run();}",
    "group": "negative",
    "name": "count_literal_range",
    "source": "fn run()->i32{let x=1u8<<256u8;7}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn run()->i32{let mut x=1u8;x<<=3u16;x>>=1i8;if x==4u8{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "legalunsupported",
    "name": "compound_shift_assignment",
    "source": "fn run()->i32{let mut x=1u8;x<<=3u16;x>>=1i8;if x==4u8{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "shift oracle ok\n",
    "full_source": "fn run()->i32{let x=1u128<<100u8;let y=x>>100u8;if y==1u128{7}else{0}}\nfn main(){if run()==7{println!(\"shift oracle ok\");}}",
    "group": "legalunsupported",
    "name": "integer128_shifts",
    "source": "fn run()->i32{let x=1u128<<100u8;let y=x>>100u8;if y==1u128{7}else{0}}"
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
    row = run_case(case, *args)
    return stock_negative_phases(case, row, args[1], args[3], args[4])


def stock_case(case, *args):
    if case.kind == "accepted_trap101":
        return stock_trap_case(case, *args)
    row = ordinary_stock_case(case, *args)
    return stock_negative_phases(case, row, args[1], args[2], args[3])


FIXTURE_BY_NAME = {fixture["name"]: fixture for fixture in FIXTURES}
STATIC_INVALIDS = {name: fixture for name, fixture in FIXTURE_BY_NAME.items()
                  if fixture.get("stock_check_status") == 0}
assert set(STATIC_INVALIDS) == {"constant_width_overflow", "constant_negative_count"}


def stock_negative_phases(case, row, directory, rustc, compile_timeout):
    """Stock metadata may accept two static errors that stock BUILD must reject.

    Native BUILD/CHECK must still reject every invalid case via shared run_case.
    No metadata success can excuse a failed BUILD classification or vice versa.
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
    expected_status = fixture.get("stock_check_status", 1)
    row["expected_stock_check_status"] = expected_status
    result = row["stock_check"]
    if expected_status == 0:
        phase_ok = (accepted(result) and metadata.is_file()
                    and set(directory.iterdir()) == before_check | {metadata})
    else:
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
            prefix="jr-shifts-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "shifts-v1",
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
            raise SystemExit(f"{totals['failure']} shifts discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
