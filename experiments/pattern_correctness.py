#!/usr/bin/env python3
"""Independent bounded pattern differential corpus, not a compatibility/speed claim.

Reuse the file-backed, deadline-bound module harness. Every fixture is first
classified by exact stock acceptance and expected runtime output. Legal features
outside this pattern slice are boundaries, never invalid Rust or parity. Native
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
            "name": "tuple_bindings_and_per_binding_mut",
            "value": 37,
            "source": "fn run() -> i32 { let (a, mut b) = (3, 4); b = b + 3; a * 10 + b }"
        },
        {
            "name": "nested_tuple_patterns",
            "value": 12,
            "source": "fn run() -> i32 { let ((a, _), (mut b, (c,))) = ((2, true), (3, (4,))); b = b + 3; a + b + c }"
        },
        {
            "name": "struct_renamed_bindings",
            "value": 37,
            "source": "struct Pair { left: i32, right: i32 } fn run() -> i32 { let Pair { right: b, left: a } = Pair { left: 3, right: 7 }; a * 10 + b }"
        },
        {
            "name": "struct_shorthand_per_binding_mut",
            "value": 37,
            "source": "struct Pair { left: i32, right: i32 } fn run() -> i32 { let Pair { left, mut right } = Pair { left: 3, right: 4 }; right = right + 3; left * 10 + right }"
        },
        {
            "name": "nested_struct_tuple_patterns",
            "value": 22,
            "source": "struct Leaf { n: i32 } struct Root { pair: (Leaf, i32), tail: Leaf } fn run() -> i32 { let root = Root { pair: (Leaf { n: 4 }, 5), tail: Leaf { n: 12 } }; let Root { pair: (Leaf { n: a }, mut b), tail: Leaf { n: c } } = root; b = b + 1; a + b + c }"
        },
        {
            "name": "tuple_noncopy_binding_moves_only_selected_field",
            "value": 12,
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = (Item { n: 5 }, Item { n: 7 }); let (a, _) = pair; consume(a) + consume(pair.1) }"
        },
        {
            "name": "struct_noncopy_binding_moves_only_selected_field",
            "value": 12,
            "source": "struct Item { n: i32 } struct Pair { a: Item, b: Item } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = Pair { a: Item { n: 5 }, b: Item { n: 7 } }; let Pair { a, b: _ } = pair; consume(a) + consume(pair.b) }"
        },
        {
            "name": "wildcard_binding_does_not_move_whole_value",
            "value": 7,
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 7 }; let _ = x; consume(x) }"
        },
        {
            "name": "wildcard_binding_does_not_read_moved_value",
            "value": 7,
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 7 }; let n = consume(x); let _ = x; n }"
        },
        {
            "name": "tuple_wildcard_skips_moved_field",
            "value": 12,
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = (Item { n: 5 }, Item { n: 7 }); let a = pair.0; let (_, b) = pair; consume(a) + consume(b) }"
        },
        {
            "name": "struct_wildcard_skips_moved_field",
            "value": 12,
            "source": "struct Item { n: i32 } struct Pair { a: Item, b: Item } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = Pair { a: Item { n: 5 }, b: Item { n: 7 } }; let a = pair.a; let Pair { a: _, b } = pair; consume(a) + consume(b) }"
        },
        {
            "name": "all_wildcards_ignore_wholly_moved_tuple",
            "value": 12,
            "source": "struct Item { n: i32 } fn consume(x: (Item, Item)) -> i32 { x.0.n + x.1.n } fn run() -> i32 { let pair = (Item { n: 5 }, Item { n: 7 }); let n = consume(pair); let (_, _) = pair; n }"
        },
        {
            "name": "all_wildcards_ignore_wholly_moved_struct",
            "value": 12,
            "source": "struct Pair { a: i32, b: i32 } fn consume(x: Pair) -> i32 { x.a + x.b } fn run() -> i32 { let pair = Pair { a: 5, b: 7 }; let n = consume(pair); let Pair { a: _, b: _ } = pair; n }"
        },
        {
            "name": "scalar_pattern_bindings_copy_without_moving_parent",
            "value": 24,
            "source": "struct Pair { a: i32, b: i32 } fn consume(x: Pair) -> i32 { x.a + x.b } fn run() -> i32 { let pair = Pair { a: 5, b: 7 }; let Pair { a, b } = pair; a + b + consume(pair) }"
        },
        {
            "name": "pattern_initializer_evaluated_once",
            "value": 1222,
            "source": "fn run() -> i32 { let mut order = 0; let (a, b) = { order = order + 1; (order, { order = order * 10 + 2; order }) }; order * 100 + a * 10 + b }"
        },
        {
            "name": "struct_initializer_source_order_not_pattern_order",
            "value": 1321,
            "source": "struct Pair { left: i32, right: i32 } fn run() -> i32 { let mut order = 0; let Pair { left: a, right: b } = Pair { right: { order = order * 10 + 1; order }, left: { order = order * 10 + 2; order } }; order * 100 + a * 10 + b }"
        },
        {
            "name": "wildcard_initializer_still_runs_side_effects",
            "value": 1,
            "source": "fn run() -> i32 { let mut n = 0; let _ = { n = n + 1; (3, 4) }; n }"
        },
        {
            "name": "shadowing_initializer_uses_old_bindings",
            "value": 55,
            "source": "fn run() -> i32 { let (x, y) = (1, 2); let (x, y) = (y + 3, x + 4); x * 10 + y }"
        },
        {
            "name": "nested_scope_pattern_shadowing",
            "value": 57,
            "source": "fn run() -> i32 { let (x, y) = (5, 7); { let (x, y) = (1, 2); let _ = x + y; } x * 10 + y }"
        },
        {
            "name": "mutable_binding_reinitializes_moved_field",
            "value": 12,
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let (mut x, _) = (Item { n: 5 }, 0); let n = consume(x); x = Item { n: 7 }; n + consume(x) }"
        },
        {
            "name": "annotated_tuple_pattern",
            "value": 7,
            "source": "fn run() -> i32 { let (a, b): (i32, i32) = (3, 4); a + b }"
        },
        {
            "name": "qualified_struct_pattern_use_alias",
            "value": 7,
            "source": "mod inner { pub struct Item { pub n: i32 } } use inner::Item as Alias; fn run() -> i32 { let Alias { n } = inner::Item { n: 7 }; n }"
        },
        {
            "name": "struct_rest_omits_private_field",
            "value": 7,
            "source": "mod inner { pub struct Item { pub n: i32, secret: i32 } pub fn make() -> Item { Item { n: 7, secret: 9 } } } fn run() -> i32 { let inner::Item { n, .. } = inner::make(); n }"
        },
        {
            "name": "struct_rest_does_not_move_omitted_subtree",
            "value": 12,
            "source": "struct Item { n: i32 } struct Pair { a: Item, b: Item } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = Pair { a: Item { n: 5 }, b: Item { n: 7 } }; let Pair { a, .. } = pair; consume(a) + consume(pair.b) }"
        },
        {
            "name": "struct_rest_skips_moved_subtree",
            "value": 12,
            "source": "struct Item { n: i32 } struct Pair { a: Item, b: Item } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = Pair { a: Item { n: 5 }, b: Item { n: 7 } }; let a = pair.a; let Pair { b, .. } = pair; consume(a) + consume(b) }"
        },
        {
            "name": "nested_wildcards_skip_moved_ancestors",
            "value": 12,
            "source": "struct Item { n: i32 } struct Pair { a: Item, b: Item } fn consume(x: Pair) -> i32 { x.a.n + x.b.n } fn run() -> i32 { let pair = Pair { a: Item { n: 5 }, b: Item { n: 7 } }; let n = consume(pair); let Pair { a: Item { n: _ }, b: _ } = pair; n }"
        },
        {
            "name": "annotated_struct_pattern",
            "value": 7,
            "source": "struct Item { n: i32 } fn run() -> i32 { let Item { n }: Item = Item { n: 7 }; n }"
        },
        {
            "name": "pattern_partial_move_then_source_field_reinit",
            "value": 14,
            "source": "struct Item { n: i32 } fn consume(x: (Item, Item)) -> i32 { x.0.n + x.1.n } fn run() -> i32 { let mut pair = (Item { n: 1 }, Item { n: 7 }); let (old, _) = pair; pair.0 = Item { n: 6 }; old.n + consume(pair) }"
        },
        {
            "name": "parenthesized_wildcard_source_does_not_read_moved_value",
            "value": 7,
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 7 }; let n = consume(x); let _ = (x); n }"
        },
        {
            "name": "self_shadowing_noncopy_pattern_uses_old_rhs",
            "value": 7,
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let s = (Item { n: 7 }, 0); let (s, _) = s; consume(s) }"
        },
        {
            "name": "pattern_bindings_fresh_each_loop_lifetime",
            "value": 9,
            "source": "fn run() -> i32 { let mut i = 0; let mut sum = 0; while i < 3 { let (x, mut y) = (1, 1); y = y + 1; sum = sum + x + y; i = i + 1; } sum }"
        }
    ],
    "negative": [
        {
            "name": "tuple_duplicate_binding",
            "code": "E0416",
            "source": "fn run() -> i32 { let (x, x) = (1, 2); x }"
        },
        {
            "name": "nested_duplicate_binding",
            "code": "E0416",
            "source": "fn run() -> i32 { let ((x, _), x) = ((1, 2), 3); x }"
        },
        {
            "name": "struct_duplicate_binding",
            "code": "E0416",
            "source": "struct Pair { a: i32, b: i32 } fn run() -> i32 { let Pair { a: x, b: x } = Pair { a: 1, b: 2 }; x }"
        },
        {
            "name": "struct_duplicate_field",
            "code": "E0025",
            "source": "struct Item { n: i32 } fn run() -> i32 { let Item { n: a, n: b } = Item { n: 1 }; a + b }"
        },
        {
            "name": "tuple_pattern_shape_mismatch",
            "code": "E0308",
            "source": "fn run() -> i32 { let (a, b) = (1, 2, 3); a + b }"
        },
        {
            "name": "tuple_pattern_on_scalar",
            "code": "E0308",
            "source": "fn run() -> i32 { let (_, _) = 3; 0 }"
        },
        {
            "name": "struct_pattern_nominal_type_mismatch",
            "code": "E0308",
            "source": "struct A { n: i32 } struct B { n: i32 } fn run() -> i32 { let A { n } = B { n: 1 }; n }"
        },
        {
            "name": "struct_pattern_missing_field",
            "code": "E0027",
            "source": "struct Pair { a: i32, b: i32 } fn run() -> i32 { let Pair { a } = Pair { a: 1, b: 2 }; a }"
        },
        {
            "name": "struct_pattern_unknown_field",
            "code": "E0026",
            "source": "struct Item { n: i32 } fn run() -> i32 { let Item { n, nope: _ } = Item { n: 1 }; n }"
        },
        {
            "name": "struct_pattern_private_field_even_wildcard",
            "code": "E0451",
            "source": "mod inner { pub struct Item { n: i32 } pub fn make() -> Item { Item { n: 1 } } } fn run() -> i32 { let inner::Item { n: _ } = inner::make(); 0 }"
        },
        {
            "name": "immutable_pattern_binding_reassignment",
            "code": "E0384",
            "source": "fn run() -> i32 { let (a, mut b) = (1, 2); a = 3; b = 4; a + b }"
        },
        {
            "name": "immutable_noncopy_binding_reassignment_after_move",
            "code": "E0384",
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let (x, _) = (Item { n: 1 }, 0); let n = consume(x); x = Item { n: 2 }; n + consume(x) }"
        },
        {
            "name": "partial_pattern_move_invalidates_parent",
            "code": "E0382",
            "source": "struct Item { n: i32 } fn consume(x: (Item, Item)) -> i32 { x.0.n + x.1.n } fn run() -> i32 { let pair = (Item { n: 1 }, Item { n: 2 }); let (a, _) = pair; a.n + consume(pair) }"
        },
        {
            "name": "pattern_binding_reads_moved_field",
            "code": "E0382",
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let pair = (Item { n: 1 }, Item { n: 2 }); let first = pair.0; let (again, _) = pair; consume(first) + consume(again) }"
        },
        {
            "name": "underscore_named_binding_is_not_wildcard",
            "code": "E0382",
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 1 }; let _ignored = x; consume(x) }"
        },
        {
            "name": "struct_pattern_private_field_explicit_wildcard_with_rest",
            "code": "E0451",
            "source": "mod inner { pub struct Item { pub n: i32, secret: i32 } pub fn make() -> Item { Item { n: 7, secret: 9 } } } fn run() -> i32 { let inner::Item { secret: _, .. } = inner::make(); 0 }"
        },
        {
            "name": "block_snapshot_wildcard_still_reads_moved_source",
            "code": "E0382",
            "source": "struct Item { n: i32 } fn consume(x: Item) -> i32 { x.n } fn run() -> i32 { let x = Item { n: 7 }; let n = consume(x); let _ = { x }; n }"
        },
        {
            "name": "pattern_initializer_cannot_see_new_binding",
            "code": "E0425",
            "source": "fn run() -> i32 { let (x, y) = (1, x); x + y }"
        },
        {
            "name": "struct_rest_only_trailing_comma",
            "code": None,
            "message": "expected `}`, found `,`",
            "source": "struct S { x: i32 } fn run() -> i32 { let s = S { x: 1 }; let S { .., } = s; 0 }"
        },
        {
            "name": "struct_rest_after_field_trailing_comma",
            "code": None,
            "message": "expected `}`, found `,`",
            "source": "struct S { x: i32 } fn run() -> i32 { let s = S { x: 1 }; let S { x, .., } = s; 0 }"
        }
    ],
    "legal_unsupported": [
        {
            "name": "tuple_rest_pattern",
            "value": 1,
            "source": "fn run() -> i32 { let (a, ..) = (1, 2, 3); a }"
        },
        {
            "name": "at_binding_pattern",
            "value": 6,
            "source": "fn run() -> i32 { let whole @ (a, b) = (1, 2); whole.0 + whole.1 + a + b }"
        },
        {
            "name": "reference_binding_pattern",
            "value": 1,
            "source": "fn run() -> i32 { let (ref a, _) = (1, 2); *a }"
        },
        {
            "name": "function_parameter_pattern",
            "value": 3,
            "source": "fn add((a, b): (i32, i32)) -> i32 { a + b } fn run() -> i32 { add((1, 2)) }"
        },
        {
            "name": "destructuring_assignment",
            "value": 7,
            "source": "fn run() -> i32 { let (mut a, mut b) = (1, 2); (a, b) = (3, 4); a + b }"
        },
        {
            "name": "let_else_pattern",
            "value": 3,
            "source": "fn run() -> i32 { let (a, b) = (1, 2) else { return 0; }; a + b }"
        }
    ]
}


def corpus():
    """Frozen stock-classified fixtures shared with tests/patterns.rs.

    Irrefutable let patterns and struct rest are supported.
    Other legal pattern syntax is classified explicitly as boundaries.
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
    assert len(cases) == 57 and len({c.name for c in cases}) == len(cases)
    corpus_hash = hashlib.sha256(json.dumps(
        [c.__dict__ for c in cases], sort_keys=True).encode()).hexdigest()
    with args.output.open("x", encoding="utf-8") as out, tempfile.TemporaryDirectory(
            prefix="jr-pattern-differential-", dir=os.environ.get("JCODE_SCRATCH_DIR")) as temporary:
        directory = Path(temporary)
        def record(row):
            out.write(json.dumps(row, sort_keys=True) + "\n")
            out.flush()
        version = process([rustc, "-Vv"], directory, args.compile_timeout)
        if not accepted(version):
            record({"record": "environment_failure", "reference": version})
            raise SystemExit("stock rustc version check failed")
        record({"record": "environment", "schema": 1, "corpus": "patterns-v1",
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
            raise SystemExit(f"{totals['failure']} pattern discrepancies, exact sources/diagnostics in {args.output}")


if __name__ == "__main__":
    main()
