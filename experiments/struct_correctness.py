#!/usr/bin/env python3
"""Independent bounded struct differential corpus, not a compatibility/speed claim.

Reuse the file-backed, deadline-bound module harness. Every fixture is first
classified by exact stock acceptance and expected runtime output. Legal features
outside this struct slice are boundaries, never invalid Rust or parity. Native
runs cover both check/build and workers 1/8. --stock-only validates the oracle
corpus before an experimental binary is available. No generated compiler code
or compiler delegation is part of the compiler under test.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import tempfile

from module_correctness import Case, accepted, process, rejected, run_case


FIXTURES = {
    "positive": [
        {
            "name": "reordered_fields",
            "value": 32,
            "source": "struct Pair { left: i32, right: i32 } fn run() -> i32 { let p = Pair { right: 2, left: 3 }; p.left * 10 + p.right }"
        },
        {
            "name": "nested_tuple_struct",
            "value": 21,
            "source": "struct Leaf { n: i32 } struct Root { pair: (Leaf, i32), tail: Leaf } fn run() -> i32 { let r = Root { pair: (Leaf { n: 4 }, 5), tail: Leaf { n: 12 } }; r.pair.0.n + r.pair.1 + r.tail.n }"
        },
        {
            "name": "scalar_field_copy_preserves_parent",
            "value": 12,
            "source": "struct Leaf { n: i32 } fn consume(x: Leaf) -> i32 { x.n } fn run() -> i32 { let x = Leaf { n: 6 }; let n = x.n; n + consume(x) }"
        },
        {
            "name": "noncopy_subtree_move_preserves_sibling",
            "value": 12,
            "source": "struct Leaf { n: i32 } struct Root { a: Leaf, b: Leaf } fn consume(x: Leaf) -> i32 { x.n } fn run() -> i32 { let r = Root { a: Leaf { n: 5 }, b: Leaf { n: 7 } }; let a = r.a; consume(a) + consume(r.b) }"
        },
        {
            "name": "partial_field_reinit_restores_whole",
            "value": 15,
            "source": "struct Leaf { n: i32 } struct Root { a: Leaf, b: Leaf } fn consume(x: Root) -> i32 { x.a.n + x.b.n } fn run() -> i32 { let mut r = Root { a: Leaf { n: 1 }, b: Leaf { n: 8 } }; let a = r.a; r.a = Leaf { n: 6 }; a.n + consume(r) }"
        },
        {
            "name": "whole_reinit_after_move",
            "value": 9,
            "source": "struct Leaf { n: i32 } fn consume(x: Leaf) -> i32 { x.n } fn run() -> i32 { let mut x = Leaf { n: 4 }; let old = consume(x); x = Leaf { n: 5 }; old + consume(x) }"
        },
        {
            "name": "nested_tuple_partial_reinit",
            "value": 10,
            "source": "struct Leaf { n: i32 } fn consume(x: (Leaf, Leaf)) -> i32 { x.0.n + x.1.n } fn run() -> i32 { let mut p = (Leaf { n: 1 }, Leaf { n: 3 }); let old = p.0; p.0 = Leaf { n: 6 }; old.n + consume(p) }"
        },
        {
            "name": "branch_moves_reinit",
            "value": 9,
            "source": "struct Leaf { n: i32 } fn consume(x: Leaf) -> i32 { x.n } fn choose(flag: bool) -> i32 { let mut x = Leaf { n: 1 }; if flag { let old = consume(x); x = Leaf { n: old + 2 }; } else { x.n = 6; } consume(x) } fn run() -> i32 { choose(true) + choose(false) }"
        },
        {
            "name": "loop_move_reinit",
            "value": 4,
            "source": "struct Leaf { n: i32 } fn consume(x: Leaf) -> i32 { x.n } fn run() -> i32 { let mut x = Leaf { n: 1 }; let mut i = 0; while i < 3 { let n = consume(x); x = Leaf { n: n + 1 }; i = i + 1; } consume(x) }"
        },
        {
            "name": "alias_constructor",
            "value": 7,
            "source": "struct Item { n: i32 } use Item as Alias; fn run() -> i32 { let x = Alias { n: 7 }; x.n }"
        },
        {
            "name": "public_module_fields",
            "value": 7,
            "source": "mod inner { pub struct Item { pub n: i32 } } fn run() -> i32 { let x = inner::Item { n: 7 }; x.n }"
        },
        {
            "name": "private_field_via_factory",
            "value": 8,
            "source": "mod inner { pub struct Item { n: i32 } pub fn make() -> Item { Item { n: 8 } } pub fn read(x: Item) -> i32 { x.n } } fn run() -> i32 { inner::read(inner::make()) }"
        },
        {
            "name": "type_value_namespace",
            "value": 12,
            "source": "struct Item { n: i32 } fn run() -> i32 { let Item = 5; let x = Item { n: 7 }; Item + x.n }"
        },
        {
            "name": "distinct_qualified_struct_names",
            "value": 12,
            "source": "mod a { pub struct Item { pub n: i32 } } mod b { pub struct Item { pub n: i32 } } fn run() -> i32 { let x = a::Item { n: 5 }; let y = b::Item { n: 7 }; x.n + y.n }"
        },
        {
            "name": "source_order_initializers",
            "value": 1321,
            "source": "struct Pair { left: i32, right: i32 } fn run() -> i32 { let mut n = 0; let p = Pair { right: { n = n * 10 + 1; n }, left: { n = n * 10 + 2; n } }; n * 100 + p.left * 10 + p.right }"
        },
        {
            "name": "immutable_new_loop_lifetime",
            "value": 6,
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let mut i = 0; let mut sum = 0; while i < 3 { let x = Item { n: 2 }; sum = sum + consume(x); i = i + 1; } sum }"
        },
        {
            "name": "wide130_fields_shallow_depth",
            "value": 193,
            "source": "struct Leaf { n: i32 } struct Wide { f0: Leaf, f1: Leaf, f2: Leaf, f3: Leaf, f4: Leaf, f5: Leaf, f6: Leaf, f7: Leaf, f8: Leaf, f9: Leaf, f10: Leaf, f11: Leaf, f12: Leaf, f13: Leaf, f14: Leaf, f15: Leaf, f16: Leaf, f17: Leaf, f18: Leaf, f19: Leaf, f20: Leaf, f21: Leaf, f22: Leaf, f23: Leaf, f24: Leaf, f25: Leaf, f26: Leaf, f27: Leaf, f28: Leaf, f29: Leaf, f30: Leaf, f31: Leaf, f32: Leaf, f33: Leaf, f34: Leaf, f35: Leaf, f36: Leaf, f37: Leaf, f38: Leaf, f39: Leaf, f40: Leaf, f41: Leaf, f42: Leaf, f43: Leaf, f44: Leaf, f45: Leaf, f46: Leaf, f47: Leaf, f48: Leaf, f49: Leaf, f50: Leaf, f51: Leaf, f52: Leaf, f53: Leaf, f54: Leaf, f55: Leaf, f56: Leaf, f57: Leaf, f58: Leaf, f59: Leaf, f60: Leaf, f61: Leaf, f62: Leaf, f63: Leaf, f64: Leaf, f65: Leaf, f66: Leaf, f67: Leaf, f68: Leaf, f69: Leaf, f70: Leaf, f71: Leaf, f72: Leaf, f73: Leaf, f74: Leaf, f75: Leaf, f76: Leaf, f77: Leaf, f78: Leaf, f79: Leaf, f80: Leaf, f81: Leaf, f82: Leaf, f83: Leaf, f84: Leaf, f85: Leaf, f86: Leaf, f87: Leaf, f88: Leaf, f89: Leaf, f90: Leaf, f91: Leaf, f92: Leaf, f93: Leaf, f94: Leaf, f95: Leaf, f96: Leaf, f97: Leaf, f98: Leaf, f99: Leaf, f100: Leaf, f101: Leaf, f102: Leaf, f103: Leaf, f104: Leaf, f105: Leaf, f106: Leaf, f107: Leaf, f108: Leaf, f109: Leaf, f110: Leaf, f111: Leaf, f112: Leaf, f113: Leaf, f114: Leaf, f115: Leaf, f116: Leaf, f117: Leaf, f118: Leaf, f119: Leaf, f120: Leaf, f121: Leaf, f122: Leaf, f123: Leaf, f124: Leaf, f125: Leaf, f126: Leaf, f127: Leaf, f128: Leaf, f129: Leaf } fn run() -> i32 { let x = Wide { f0: Leaf { n: 0 }, f1: Leaf { n: 1 }, f2: Leaf { n: 2 }, f3: Leaf { n: 3 }, f4: Leaf { n: 4 }, f5: Leaf { n: 5 }, f6: Leaf { n: 6 }, f7: Leaf { n: 7 }, f8: Leaf { n: 8 }, f9: Leaf { n: 9 }, f10: Leaf { n: 10 }, f11: Leaf { n: 11 }, f12: Leaf { n: 12 }, f13: Leaf { n: 13 }, f14: Leaf { n: 14 }, f15: Leaf { n: 15 }, f16: Leaf { n: 16 }, f17: Leaf { n: 17 }, f18: Leaf { n: 18 }, f19: Leaf { n: 19 }, f20: Leaf { n: 20 }, f21: Leaf { n: 21 }, f22: Leaf { n: 22 }, f23: Leaf { n: 23 }, f24: Leaf { n: 24 }, f25: Leaf { n: 25 }, f26: Leaf { n: 26 }, f27: Leaf { n: 27 }, f28: Leaf { n: 28 }, f29: Leaf { n: 29 }, f30: Leaf { n: 30 }, f31: Leaf { n: 31 }, f32: Leaf { n: 32 }, f33: Leaf { n: 33 }, f34: Leaf { n: 34 }, f35: Leaf { n: 35 }, f36: Leaf { n: 36 }, f37: Leaf { n: 37 }, f38: Leaf { n: 38 }, f39: Leaf { n: 39 }, f40: Leaf { n: 40 }, f41: Leaf { n: 41 }, f42: Leaf { n: 42 }, f43: Leaf { n: 43 }, f44: Leaf { n: 44 }, f45: Leaf { n: 45 }, f46: Leaf { n: 46 }, f47: Leaf { n: 47 }, f48: Leaf { n: 48 }, f49: Leaf { n: 49 }, f50: Leaf { n: 50 }, f51: Leaf { n: 51 }, f52: Leaf { n: 52 }, f53: Leaf { n: 53 }, f54: Leaf { n: 54 }, f55: Leaf { n: 55 }, f56: Leaf { n: 56 }, f57: Leaf { n: 57 }, f58: Leaf { n: 58 }, f59: Leaf { n: 59 }, f60: Leaf { n: 60 }, f61: Leaf { n: 61 }, f62: Leaf { n: 62 }, f63: Leaf { n: 63 }, f64: Leaf { n: 64 }, f65: Leaf { n: 65 }, f66: Leaf { n: 66 }, f67: Leaf { n: 67 }, f68: Leaf { n: 68 }, f69: Leaf { n: 69 }, f70: Leaf { n: 70 }, f71: Leaf { n: 71 }, f72: Leaf { n: 72 }, f73: Leaf { n: 73 }, f74: Leaf { n: 74 }, f75: Leaf { n: 75 }, f76: Leaf { n: 76 }, f77: Leaf { n: 77 }, f78: Leaf { n: 78 }, f79: Leaf { n: 79 }, f80: Leaf { n: 80 }, f81: Leaf { n: 81 }, f82: Leaf { n: 82 }, f83: Leaf { n: 83 }, f84: Leaf { n: 84 }, f85: Leaf { n: 85 }, f86: Leaf { n: 86 }, f87: Leaf { n: 87 }, f88: Leaf { n: 88 }, f89: Leaf { n: 89 }, f90: Leaf { n: 90 }, f91: Leaf { n: 91 }, f92: Leaf { n: 92 }, f93: Leaf { n: 93 }, f94: Leaf { n: 94 }, f95: Leaf { n: 95 }, f96: Leaf { n: 96 }, f97: Leaf { n: 97 }, f98: Leaf { n: 98 }, f99: Leaf { n: 99 }, f100: Leaf { n: 100 }, f101: Leaf { n: 101 }, f102: Leaf { n: 102 }, f103: Leaf { n: 103 }, f104: Leaf { n: 104 }, f105: Leaf { n: 105 }, f106: Leaf { n: 106 }, f107: Leaf { n: 107 }, f108: Leaf { n: 108 }, f109: Leaf { n: 109 }, f110: Leaf { n: 110 }, f111: Leaf { n: 111 }, f112: Leaf { n: 112 }, f113: Leaf { n: 113 }, f114: Leaf { n: 114 }, f115: Leaf { n: 115 }, f116: Leaf { n: 116 }, f117: Leaf { n: 117 }, f118: Leaf { n: 118 }, f119: Leaf { n: 119 }, f120: Leaf { n: 120 }, f121: Leaf { n: 121 }, f122: Leaf { n: 122 }, f123: Leaf { n: 123 }, f124: Leaf { n: 124 }, f125: Leaf { n: 125 }, f126: Leaf { n: 126 }, f127: Leaf { n: 127 }, f128: Leaf { n: 128 }, f129: Leaf { n: 129 } }; x.f0.n + x.f64.n + x.f129.n }"
        },
        {
            "name": "if_result_transfers_noncopy_value",
            "value": 11,
            "source": "struct Item { n: i32 } fn choose(flag: bool, x: Item) -> Item { if flag { x } else { Item { n: 7 } } } fn run() -> i32 { let first = choose(true, Item { n: 4 }); let second = choose(false, Item { n: 5 }); first.n + second.n }"
        },
        {
            "name": "loop_break_transfers_noncopy_value",
            "value": 9,
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 9 }; let y = loop { break x; }; consume(y) }"
        }
    ],
    "negative": [
        {
            "name": "whole_move_second_use",
            "code": "E0382",
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 1 }; let a = consume(x); a + consume(x) }"
        },
        {
            "name": "tuple_containing_struct_not_copy",
            "code": "E0382",
            "source": "struct Item { n: i32 } fn consume(x: (Item, i32)) -> i32 { x.0.n + x.1 } fn run() -> i32 { let x = (Item { n: 1 }, 2); let y = x; consume(y) + consume(x) }"
        },
        {
            "name": "partial_move_whole_read",
            "code": "E0382",
            "source": "struct Item { n: i32 } struct Root { a: Item, b: Item } fn consume(x: Root) -> i32 { x.b.n } fn run() -> i32 { let x = Root { a: Item { n: 1 }, b: Item { n: 2 } }; let a = x.a; a.n + consume(x) }"
        },
        {
            "name": "partial_move_same_subtree_twice",
            "code": "E0382",
            "source": "struct Item { n: i32 } struct Root { a: Item, b: Item } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Root { a: Item { n: 1 }, b: Item { n: 2 } }; let a = x.a; consume(a) + consume(x.a) }"
        },
        {
            "name": "immutable_reassignment_after_move",
            "code": "E0384",
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 1 }; let n = consume(x); x = Item { n: 2 }; n + consume(x) }"
        },
        {
            "name": "immutable_field_write",
            "code": "E0594",
            "source": "struct Item { n: i32 } fn run() -> i32 { let x = Item { n: 1 }; x.n = 2; x.n }"
        },
        {
            "name": "move_on_branch_join",
            "code": "E0382",
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn choose(flag: bool) -> i32 { let x = Item { n: 1 }; if flag { consume(x); } else { } consume(x) } fn run() -> i32 { choose(false) }"
        },
        {
            "name": "loop_consumes_without_reinit",
            "code": "E0382",
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 1 }; let mut i = 0; while i < 3 { consume(x); i = i + 1; } 0 }"
        },
        {
            "name": "nominal_identical_shape_mismatch",
            "code": "E0308",
            "source": "struct A { n: i32 } struct B { n: i32 } fn consume(x: A) -> i32 { x.n } fn run() -> i32 { consume(B { n: 1 }) }"
        },
        {
            "name": "missing_field",
            "code": "E0063",
            "source": "struct Item { a: i32, b: i32 } fn run() -> i32 { let x = Item { a: 1 }; x.a }"
        },
        {
            "name": "duplicate_field",
            "code": "E0062",
            "source": "struct Item { n: i32 } fn run() -> i32 { let x = Item { n: 1, n: 2 }; x.n }"
        },
        {
            "name": "unknown_field_constructor",
            "code": "E0560",
            "source": "struct Item { n: i32 } fn run() -> i32 { let x = Item { nope: 1 }; x.n }"
        },
        {
            "name": "unknown_field_read",
            "code": "E0609",
            "source": "struct Item { n: i32 } fn run() -> i32 { let x = Item { n: 1 }; x.nope }"
        },
        {
            "name": "field_type_mismatch",
            "code": "E0308",
            "source": "struct Item { n: i32 } fn run() -> i32 { let x = Item { n: true }; x.n }"
        },
        {
            "name": "private_field_read",
            "code": "E0616",
            "source": "mod inner { pub struct Item { n: i32 } pub fn make() -> Item { Item { n: 1 } } } fn run() -> i32 { inner::make().n }"
        },
        {
            "name": "private_field_construction",
            "code": "E0451",
            "source": "mod inner { pub struct Item { n: i32 } } fn run() -> i32 { let x = inner::Item { n: 1 }; 0 }"
        },
        {
            "name": "direct_recursive_by_value",
            "code": "E0072",
            "source": "struct Item { next: Item } fn run() -> i32 { 0 }"
        },
        {
            "name": "mutual_recursive_by_value",
            "code": "E0072",
            "source": "struct A { b: B } struct B { a: A } fn run() -> i32 { 0 }"
        },
        {
            "name": "tuple_recursive_by_value",
            "code": "E0072",
            "source": "struct Item { next: (i32, Item) } fn run() -> i32 { 0 }"
        },
        {
            "name": "block_snapshot_projection_moves_whole_parent",
            "code": "E0382",
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 3 }; let n = { x }.n; n + consume(x) }"
        }
    ],
    "legal_unsupported": [
        {
            "name": "empty_braced_struct",
            "value": 1,
            "source": "struct Empty {} fn run() -> i32 { let x = Empty {}; 1 }"
        },
        {
            "name": "type_alias_constructor",
            "value": 7,
            "source": "struct Item { n: i32 } type Alias = Item; fn run() -> i32 { let x = Alias { n: 7 }; x.n }"
        }
    ]
}


def corpus():
    """Frozen stock-classified fixtures shared with tests/structs.rs.

    Import aliases are supported, type aliases remain a legal boundary.
    Each positive has a literal stdout oracle independent of native output.
    """
    for group, kind in [("positive", "valid"), ("negative", "invalid"),
                        ("legal_unsupported", "boundary")]:
        for fixture in FIXTURES[group]:
            name = fixture["name"]
            if kind == "invalid":
                source = fixture["source"] + "\nfn main() { run(); }"
                yield Case(name, kind, source)
            else:
                source = fixture["source"] + (
                    f'\nfn main() {{ if run() == {fixture["value"]} {{ println!("{name}"); }} }}')
                yield Case(name, kind, source, name + "\n")


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
    assert len(cases) == 41 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-struct-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "structs-v1",
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
            raise SystemExit(f"{totals['failure']} struct discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
