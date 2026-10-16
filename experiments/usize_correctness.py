#!/usr/bin/env python3
"""Bounded 64-bit usize/dynamic-index oracle, not full Rust compatibility.

Exact stock-classified sources/stdout mirror tests/usize.rs. Ordinary cases use
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


ACCEPTANCE_CORPUS_SHA256 = "e5ce5df7544cba3e3ea5ccbdbb02dc0abcb714e35570e2d29d635277a318fc3c"
TRAP_POLICY = ("declared runtime_error only: compile/check0, executable required, "
               "completed runtime101 and exact stdout; stock stderr contains panicked at "
               "and fixture reason; native stderr captured but may differ or be empty; "
               "signals/timeouts/truncation/compiler crashes fail")

# Frozen bear oracle. full_source is authoritative, without additional wrappers.
FIXTURES = json.loads(r'''
[
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn identity(x:usize)->usize{x} fn run()->i32{if identity(wide())==4294967296{if identity(high())==9223372036854775808{if identity(maximum())==18446744073709551615{7i32}else{0}}else{0}}else{0}}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "full64_function_transport",
    "source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn identity(x:usize)->usize{x} fn run()->i32{if identity(wide())==4294967296{if identity(high())==9223372036854775808{if identity(maximum())==18446744073709551615{7i32}else{0}}else{0}}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} struct S{n:usize} enum E{A(usize)} fn run()->i32{let t=(wide(),maximum());let s=S{n:t.0};let a=[t.1;2];match E::A(s.n){E::A(n)=>if n==wide(){if a[1]==maximum(){7}else{0}}else{0}}}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "aggregate_usize_transport",
    "source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} struct S{n:usize} enum E{A(usize)} fn run()->i32{let t=(wide(),maximum());let s=S{n:t.0};let a=[t.1;2];match E::A(s.n){E::A(n)=>if n==wide(){if a[1]==maximum(){7}else{0}}else{0}}}"
  },
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn run()->i32{let z:usize=0;if high()>wide() && maximum()>high() && z<maximum() && maximum()>=high() && high()<=maximum() && high()!=wide() && z==z{7}else{0}}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "unsigned_comparisons",
    "source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn run()->i32{let z:usize=0;if high()>wide() && maximum()>high() && z<maximum() && maximum()>=high() && high()<=maximum() && high()!=wide() && z==z{7}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn run()->i32{let a:usize=4294967296;let b:usize=7;let x=(a+b)*b-b;let h:usize=9223372036854775807;let one:usize=1;let m:usize=4611686018427387904;let two:usize=2;if x==30064771114{if h+one==9223372036854775808{if m*two==9223372036854775808{7}else{0}}else{0}}else{0}}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "wide_checked_arithmetic",
    "source": "fn run()->i32{let a:usize=4294967296;let b:usize=7;let x=(a+b)*b-b;let h:usize=9223372036854775807;let one:usize=1;let m:usize=4611686018427387904;let two:usize=2;if x==30064771114{if h+one==9223372036854775808{if m*two==9223372036854775808{7}else{0}}else{0}}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn run()->i32{let d:usize=3;if high()/d==3074457345618258602{if high()%d==2{7}else{0}}else{0}}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "unsigned_division_remainder",
    "source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn run()->i32{let d:usize=3;if high()/d==3074457345618258602{if high()%d==2{7}else{0}}else{0}}"
  },
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn index()->usize{1} fn run()->i32{let a=[3,7];a[index()]}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "dynamic_scalar_read",
    "source": "fn index()->usize{1} fn run()->i32{let a=[3,7];a[index()]}"
  },
  {
    "expected": 37,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn index()->usize{1} fn run()->i32{let a=[(1,2),(3,7)];let p=a[index()];p.0*10+p.1}\nfn main() { if run() == 37 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "copy_tuple_dynamic_read",
    "source": "fn index()->usize{1} fn run()->i32{let a=[(1,2),(3,7)];let p=a[index()];p.0*10+p.1}"
  },
  {
    "expected": 37,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn index()->usize{1} fn run()->i32{let a=[[1,2],[3,7]];let p=a[index()];p[0]*10+p[1]}\nfn main() { if run() == 37 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "copy_array_dynamic_read",
    "source": "fn index()->usize{1} fn run()->i32{let a=[[1,2],[3,7]];let p=a[index()];p[0]*10+p[1]}"
  },
  {
    "expected": "base\nindex\n7",
    "expected_stdout": "base\nindex\ndynamic oracle ok\n",
    "full_source": "fn base()->[i32;2]{println!(\"base\");[3,7]} fn index()->usize{println!(\"index\");1} fn run()->i32{base()[index()]}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "read_base_then_index",
    "source": "fn base()->[i32;2]{println!(\"base\");[3,7]} fn index()->usize{println!(\"index\");1} fn run()->i32{base()[index()]}"
  },
  {
    "expected": "rhs\nindex\n7",
    "expected_stdout": "rhs\nindex\ndynamic oracle ok\n",
    "full_source": "fn index()->usize{println!(\"index\");1} fn rhs()->i32{println!(\"rhs\");7} fn run()->i32{let mut a=[3,4];a[index()]=rhs();a[1]}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "assignment_rhs_then_index",
    "source": "fn index()->usize{println!(\"index\");1} fn rhs()->i32{println!(\"rhs\");7} fn run()->i32{let mut a=[3,4];a[index()]=rhs();a[1]}"
  },
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn run()->i32{let mut a=[3];a[{a=[7];0}]}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "index_replaces_live_array",
    "source": "fn run()->i32{let mut a=[3];a[{a=[7];0}]}"
  },
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "struct S{n:i32} fn index()->usize{0} fn run()->i32{let mut a=[S{n:3}];let _=a[index()];a[index()]=S{n:7};a[0].n}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "noncopy_wildcard_and_replace",
    "source": "struct S{n:i32} fn index()->usize{0} fn run()->i32{let mut a=[S{n:3}];let _=a[index()];a[index()]=S{n:7};a[0].n}"
  },
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn index()->usize{0} fn run()->i32{let mut a:[i32;1];a[index()]={a=[3];7};a[0]}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "assignment_rhs_initializes_array",
    "source": "fn index()->usize{0} fn run()->i32{let mut a:[i32;1];a[index()]={a=[3];7};a[0]}"
  },
  {
    "expected": "E0277",
    "full_source": "fn index()->i32{1} fn run()->i32{let a=[3,7];a[index()]}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "i32_dynamic_index",
    "source": "fn index()->i32{1} fn run()->i32{let a=[3,7];a[index()]}"
  },
  {
    "expected": "E0508",
    "full_source": "struct S{n:i32} fn index()->usize{0} fn take(s:S)->i32{s.n} fn run()->i32{let a=[S{n:7}];take(a[index()])}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "noncopy_dynamic_move",
    "source": "struct S{n:i32} fn index()->usize{0} fn take(s:S)->i32{s.n} fn run()->i32{let a=[S{n:7}];take(a[index()])}"
  },
  {
    "expected": "E0600",
    "full_source": "fn run()->i32{let x:usize=7;let y=-x;0}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "usize_unary_negation",
    "source": "fn run()->i32{let x:usize=7;let y=-x;0}"
  },
  {
    "expected": null,
    "full_source": "fn run()->i32{let x:usize=18446744073709551616;0}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "usize_literal_overflow",
    "source": "fn run()->i32{let x:usize=18446744073709551616;0}"
  },
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn run()->i32{let a:[i32;1];let x=a[{a=[7];0}];x}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "index_initializes_uninitialized_array",
    "source": "fn run()->i32{let a:[i32;1];let x=a[{a=[7];0}];x}"
  },
  {
    "expected_exit": 101,
    "expected_stdout": "before\n",
    "full_source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn add(a:usize,b:usize)->usize{a+b} fn run()->i32{println!(\"before\");let one:usize=1;let n=add(maximum(),one);0}\nfn main() { run(); println!(\"after trap\"); }\n",
    "group": "runtime_error",
    "name": "checked_add_overflow",
    "source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn add(a:usize,b:usize)->usize{a+b} fn run()->i32{println!(\"before\");let one:usize=1;let n=add(maximum(),one);0}",
    "stock_panic_reason": "attempt to add with overflow"
  },
  {
    "expected_exit": 101,
    "expected_stdout": "before\n",
    "full_source": "fn sub(a:usize,b:usize)->usize{a-b} fn run()->i32{println!(\"before\");let zero:usize=0;let one:usize=1;let n=sub(zero,one);0}\nfn main() { run(); println!(\"after trap\"); }\n",
    "group": "runtime_error",
    "name": "checked_sub_underflow",
    "source": "fn sub(a:usize,b:usize)->usize{a-b} fn run()->i32{println!(\"before\");let zero:usize=0;let one:usize=1;let n=sub(zero,one);0}",
    "stock_panic_reason": "attempt to subtract with overflow"
  },
  {
    "expected_exit": 101,
    "expected_stdout": "before\n",
    "full_source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn mul(a:usize,b:usize)->usize{a*b} fn run()->i32{println!(\"before\");let two:usize=2;let n=mul(high(),two);0}\nfn main() { run(); println!(\"after trap\"); }\n",
    "group": "runtime_error",
    "name": "checked_mul_overflow",
    "source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn mul(a:usize,b:usize)->usize{a*b} fn run()->i32{println!(\"before\");let two:usize=2;let n=mul(high(),two);0}",
    "stock_panic_reason": "attempt to multiply with overflow"
  },
  {
    "expected_exit": 101,
    "expected_stdout": "before\n",
    "full_source": "fn divide(a:usize,b:usize)->usize{a/b} fn run()->i32{println!(\"before\");let a:usize=7;let b:usize=0;let n=divide(a,b);0}\nfn main() { run(); println!(\"after trap\"); }\n",
    "group": "runtime_error",
    "name": "usize_divide_by_zero",
    "source": "fn divide(a:usize,b:usize)->usize{a/b} fn run()->i32{println!(\"before\");let a:usize=7;let b:usize=0;let n=divide(a,b);0}",
    "stock_panic_reason": "attempt to divide by zero"
  },
  {
    "expected_exit": 101,
    "expected_stdout": "before\n",
    "full_source": "fn rem(a:usize,b:usize)->usize{a%b} fn run()->i32{println!(\"before\");let a:usize=7;let b:usize=0;let n=rem(a,b);0}\nfn main() { run(); println!(\"after trap\"); }\n",
    "group": "runtime_error",
    "name": "usize_remainder_by_zero",
    "source": "fn rem(a:usize,b:usize)->usize{a%b} fn run()->i32{println!(\"before\");let a:usize=7;let b:usize=0;let n=rem(a,b);0}",
    "stock_panic_reason": "attempt to calculate the remainder with a divisor of zero"
  },
  {
    "expected_exit": 101,
    "expected_stdout": "before\n",
    "full_source": "fn index()->usize{2} fn run()->i32{println!(\"before\");let a=[3,7];a[index()]}\nfn main() { run(); println!(\"after trap\"); }\n",
    "group": "runtime_error",
    "name": "bounds_equal_length",
    "source": "fn index()->usize{2} fn run()->i32{println!(\"before\");let a=[3,7];a[index()]}",
    "stock_panic_reason": "index out of bounds"
  },
  {
    "expected_exit": 101,
    "expected_stdout": "before\n",
    "full_source": "fn index()->usize{4294967296} fn run()->i32{println!(\"before\");let a=[3,7];a[index()]}\nfn main() { run(); println!(\"after trap\"); }\n",
    "group": "runtime_error",
    "name": "bounds_no_low32_truncation",
    "source": "fn index()->usize{4294967296} fn run()->i32{println!(\"before\");let a=[3,7];a[index()]}",
    "stock_panic_reason": "index out of bounds"
  },
  {
    "expected_exit": 101,
    "expected_stdout": "before\n",
    "full_source": "fn index()->usize{9223372036854775808} fn run()->i32{println!(\"before\");let a=[3,7];a[index()]}\nfn main() { run(); println!(\"after trap\"); }\n",
    "group": "runtime_error",
    "name": "bounds_highbit_unsigned",
    "source": "fn index()->usize{9223372036854775808} fn run()->i32{println!(\"before\");let a=[3,7];a[index()]}",
    "stock_panic_reason": "index out of bounds"
  },
  {
    "expected_exit": 101,
    "expected_stdout": "before\n",
    "full_source": "fn index()->usize{0} fn run()->i32{println!(\"before\");let a:[i32;0]=[];a[index()]}\nfn main() { run(); println!(\"after trap\"); }\n",
    "group": "runtime_error",
    "name": "bounds_empty_array",
    "source": "fn index()->usize{0} fn run()->i32{println!(\"before\");let a:[i32;0]=[];a[index()]}",
    "stock_panic_reason": "index out of bounds"
  },
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn run()->i32{let i=1;let a=[3,7];a[i]}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "backward_index_inference",
    "source": "fn run()->i32{let i=1;let a=[3,7];a[i]}"
  },
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "struct S{n:i32} fn index()->usize{0} fn run()->i32{let a=[S{n:7}];a[index()].n}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "legalunsupported",
    "name": "noncopy_dynamic_copy_field",
    "source": "struct S{n:i32} fn index()->usize{0} fn run()->i32{let a=[S{n:7}];a[index()].n}"
  },
  {
    "expected": 7,
    "expected_stdout": "dynamic oracle ok\n",
    "full_source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn pick(a:usize,b:usize,c:usize,d:usize,e:usize,f:usize,g:usize,h:usize)->usize{h} fn run()->i32{let z:usize=0;if pick(z,z,z,z,z,z,z,maximum())==maximum(){7}else{0}}\nfn main() { if run() == 7 { println!(\"dynamic oracle ok\"); } }\n",
    "group": "positive",
    "name": "usize_stack_argument_transport",
    "source": "fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn pick(a:usize,b:usize,c:usize,d:usize,e:usize,f:usize,g:usize,h:usize)->usize{h} fn run()->i32{let z:usize=0;if pick(z,z,z,z,z,z,z,maximum())==maximum(){7}else{0}}"
  },
  {
    "expected": "E0381",
    "full_source": "fn run()->i32{let a:[[i32;1];1];a[0][{a=[[7]];0}]}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "static_prefix_requires_readiness",
    "source": "fn run()->i32{let a:[[i32;1];1];a[0][{a=[[7]];0}]}"
  },
  {
    "expected": "E0308",
    "full_source": "fn run()->i32{let a:usize=7;let b:i32=3;let n=a+b;0}\nfn main() { run(); }\n",
    "group": "negative",
    "name": "mixed_concrete_arithmetic",
    "source": "fn run()->i32{let a:usize=7;let b:i32=3;let n=a+b;0}"
  }
]
''')
TRAPS = {f["name"]: f for f in FIXTURES if f["group"] == "runtime_error"}
assert len(TRAPS) == 9
assert all(f["expected_exit"] == 101 and f["stock_panic_reason"]
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
            "overflow-checks=yes", source]
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
            prefix="jr-usize-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "usize-dynamic-index-v1",
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
            raise SystemExit(f"{totals['failure']} usize/dynamic-index discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
