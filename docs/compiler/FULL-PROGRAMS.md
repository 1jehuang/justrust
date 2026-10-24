# Full programs: the target corpus

"Compile a full program" means: every program in
`tests/fixtures/full_programs/` is native-checked, built and run, and prints
exactly what stock rustc 1.98.1 (`--edition 2024 -C panic=abort`) prints.
That is the first-priority acceptance gate. Passing it is still a subset
milestone, not full Rust support (see RUST-COMPATIBILITY.md).

- 20 realistic single-file programs, std only, increasing difficulty
  (hello, fizzbuzz, collatz, sieve, gcd/fib, matrix, bank account, enum state
  machine, generic stack, string processing, closures/iterators, linked list,
  trait objects, generic containers, `Result`/`?`, sorting, HashMap/BTreeMap,
  tokenizer+calculator, operator overloading, Rc/RefCell inventory).
- `<name>.stdout` is the stock oracle output. Run `scripts/full_programs_oracle.sh`
  to verify it, or `--write` to regenerate. Stock is only an oracle. The test never runs it.
- `manifest.tsv` holds the ratchet: `pass` programs must keep passing. `fail` programs
  must be rejected with an explicit diagnostic (a wrong-output build is a
  miscompile and fails). An unexpected pass fails until you flip the line.
  The `needs` column lists missing feature tags. When a feature lands, delete its tag
  everywhere. A `pass` line must have empty needs.
- `justrust test --test full_programs -- --nocapture` prints per-program
  `PASS` or the first native diagnostic.

## Unchanged Adler2 repository probe

Run the pinned Adler2 2.0.1 target without adapting its source:

```sh
python3 experiments/adler2_repo_probe.py \
  --native ./target/debug/jr-rust \
  --output-dir "$JCODE_SCRATCH_DIR/adler2-probe-new"
```

The default input is an already-cached Cargo registry copy of Adler2 2.0.1.
Alternatively pass `--repo /path/to/adler2`; both upstream Rust files must match
its reviewed SHA-256 pins (release provenance revision
`89a031a0f42eeff31c70dc598b398cbf31f1680f`). No download happens. The runner
copies the package and supplied native binary into the new output directory;
it never edits the input repository or reuses an old output directory.

The stock oracle builds/checks the unchanged library in `no_std` and default
`std` configurations, runs upstream unit tests, and runs a separate consumer
against five checksum vectors, including incremental updates. Native workers
1 and 8 attempt the real project check/build/test interfaces and the separate
path-dependent consumer. A direct `--lib --check` attempt also captures an
independent source diagnostic. That diagnostic is not library-build success.

`report.json` records exact commands, binary/source hashes, per-stage statuses,
full diagnostics, compatibility blockers and missing-dependency blockers.
Per-command stdout/stderr files retain the raw output. Exit 0 requires every
required native stage to pass, exit 1 means compiler compatibility is blocked,
and exit 2 means an input/environment/oracle problem needs attention. A run
can expose compatibility blockers even while its overall result is
`environment_blocked`. Never treat timeouts, missing artifacts, or a successful
source-only check as a successful target build.

This is a first-blocker probe, not a complete Rust gap inventory. Later errors
can remain hidden. Doctests, benchmarks and optional internal/docsrs features
are outside this target, and the stock baseline uses rustc directly rather
than claiming stock Cargo workflow coverage. The older
`adler2_adapted_acceptance.py` remains a different, explicitly adapted test.
Runner regression tests:

```sh
python3 -m unittest discover -s experiments -p test_adler2_repo_probe.py
```

## Status (2026-10-23)

11/20 manifest entries pass, including the unchanged matrix and string-processing
programs. The full hosted corpus ratchet passed in
`20261023-031756111-273302` against the isolated staged source snapshot. The string program also passes stock-differential
CHECK/BUILD/run at workers 1 and 8. Nine entries remain explicit rejection
boundaries, so this does not complete the corpus or full Rust support.

## Historical status (2026-10-20)

The pass count below is current through the manifest ratchet. The original blocker matrix further below is a historical inventory,
not a fresh measurement of today's diagnostics. Subsequent source-prelude,
formatting, pointer, pattern, closure and borrowed-iteration slices are tracked
in `IMPLEMENTATION.md` and their scoped feature documents.

8/20 pass (adds `09_vec_stack`, `19_operator_overloading`): `01_hello`, `02_fizzbuzz`, `03_collatz`, `05_gcd_fib`,
`07_bank_account` and `08_enum_state_machine` check, build and execute with
exact stock stdout (manifest ratchet). The unchanged 08 was additionally
stock-differential CHECKed, BUILT and run at workers 1 and 8 by
`justrust test --test iterable_literals unchanged_enum_state_machine_matches_stock`.
Its immediate-for fixed-array `.iter()` path preserves real method priority,
shared element loans and exact original source-call correspondence. A full
hosted replay (`20261020-050209562-3991979`) verifies all 20 entries, with six
successful executions and 14 explicit rejections. This is not general Iterator,
full formatting or full Rust support.

## Current first rejection boundaries

Refreshed 2026-10-23 by `justrust test --test full_programs -- --nocapture`
on the exact committed borrowed-callable source snapshot
(`20261023-082306614-1289172`). All 20 entries completed: 11 executions
matched their stored stock output and nine rejected explicitly.

| Programs | First reported boundary |
|---|---|
| 11 | module authority resource boundary in the combined iterator program |
| 12 | recursive nominal field realization requires an explicit recursive layout protocol |
| 13 | trait object types only admitted directly behind a reference (`Box<dyn>`) |
| 14, 15 | unresolved nominal source constraints in `main` (15 advances past `parse_u32` after `str::is_empty`, rechecked in `20261023-083331529-1328667`) |
| 16 | no method `split_at_mut` for `&mut [T]` |
| 17 | unresolved or cyclic import `BTreeMap` |
| 18 | no method `parse` for `str` |
| 20 | unresolved or cyclic import `Rc` |

The iterator adapters now pass their scoped acceptance suites, but their combined
use in program 11 reaches an authority resource boundary. Collection libraries
remain incomplete. See HASHMAP.md for collection prerequisites (generic
`Hash<H: Hasher>`, `BuildHasher`, `Hash`/`Eq` for `str`/`String`).

These are the first diagnostics, not an exhaustive feature inventory or evidence
that fixing one diagnostic makes the program pass. In particular, most remaining
programs still need reusable library types, trait dispatch and iterator machinery.
Only stock-matching CHECK, BUILD and execution can promote a manifest entry.

An older scoped follow-up on `af5c06a69` after scalar source constants rechecked the
unchanged `06_matrix` at workers 1 and 8. Both CHECK requests reject at line 3,
column 23: `const-expression array length is unsupported; use an integer literal`.
This replaces the earlier const-item boundary for this one program, not the
other rows above. The exact source and binary hashes and exit codes are recorded
in `data/runs/const-source/matrix-boundary.json`. No BUILD, execution or stock
oracle rerun was performed for this follow-up, and the manifest then remained 3/20.

## Historical feature-demand matrix

The counts below describe features required by the original 20 programs, before
incremental implementations landed. They are **not current counts of blocked
programs**. Scalar Display, string literals, range/array loops, match ergonomics,
byte literals, primitive methods, dyn references and some Option/Result methods
now have supported slices. A broad feature tag can remain when a program needs
an unsupported part, such as generic Iterator dispatch rather than integer-range
lowering.

The quoted probes are retained historical diagnostics, not a claim that those
minimal examples still reject. Re-run the corpus for current first errors.
First-diagnostic counts also undercount remaining requirements because each
program stops at its first error.

| # | tag | original demand | feature | historical probe |
|---|-----|-------:|---------|-------------|
| 1 | fmt-display | 20 | `println!`/`print!` with `{}` args (ints, bool, char, &str, String, f64) | "formatting placeholders are unsupported" (FORMAT-SCALARS.md slice in progress) |
| 2 | for | 16 | `for` over ranges (`a..b`, `a..=b`, typed `1..10u64`) and IntoIterator | "for loops and iterators ... unsupported" |
| 3 | vec | 14 | `Vec<T>`: `Vec::new`, `vec![..]`, `vec![x; n]`, push/pop/len/index/last/insert/extend | `Vec::new` "unresolved generic declaration", `vec!` unknown macro |
| 3 | iter | 14 | Iterator adapters: iter/enumerate/map/filter/rev/zip/skip/take/step_by/sum/product/count/fold/any/all/position/max_by/collect | "no applicable method iter" |
| 5 | fmt-debug | 13 | `{:?}` and `derive(Debug)` output for ints, tuples, Vec, Option, enums, structs | unsupported placeholder |
| 6 | str | 12 | string literals as `&str` values and parameters, `len`, `==` | "expected ASCII identifier, found \"hi\"" |
| 6 | closure | 12 | closures (`\|x\| ..`, `move`, capturing, FnMut) | "expected ASCII identifier, found \"\|\"" |
| 8 | string | 10 | `String`: from, to_string, push, push_str, `==` with `&str`, join, to_uppercase | string literal blocked first |
| 9 | option-result | 9 | prelude `Option`/`Result`, `Some`/`None`/`Ok`/`Err`, unwrap/unwrap_or/map/ok_or/map_err/take/is_ok | "unresolved generic declaration" / "unresolved function" |
| 10 | derive | 7 | `#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]` | "active attribute `derive` is unsupported" |
| 10 | slice | 7 | slices beyond `&[T]` indexing: `&v[a..b]`, `iter`, `chunks`, `swap`, `split_at_mut`, `to_vec`, `binary_search`, `sort*` | partial: `fn f(v:&[i32])` + index works |
| 10 | prim-methods | 7 | inherent methods on primitives: `abs`, `wrapping_*`, `checked_*`, `sqrt`, `is_ascii_*`, `to_digit`, `is_alphanumeric`, `cmp`, `partial_cmp`, `parse` | "no applicable method abs" |
| 13 | match-ergonomics | 6 | default binding modes (`match &x { Some(v) => }`), `&p` patterns, ref bindings in by-ref matches | "borrowing requires a local-root static place" |
| 13 | array-iter | 6 | `for x in [a, b, c]` / arrays as IntoIterator | needs `for` |
| 15 | std-paths | 5 | `use std::{fmt, ops::Add, collections::HashMap, rc::Rc, cell::RefCell}` | "unresolved or cyclic import crate::fmt" |
| 16 | format-macro | 4 | `format!`, `write!` to `String`/`Formatter` | unknown macro |
| 17 | user-fmt | 3 | `impl fmt::Display` used by `{}` and `to_string()` | std-paths first |
| 17 | try | 3 | `?` on Option and Result (incl. `From` conversion) | "unsupported token ?" |
| 17 | fmt-float | 3 | f64 `{}` shortest round-trip printing, `parse::<f64>` | float arithmetic works, printing does not |
| 17 | autoderef | 3 | implicit autoderef for method calls, field access and indexing through `&`/`&&` | "implicit reference autodereference is unsupported in B1" |
| 21 | generic-bounds | 2 | generic impls with `T: PartialOrd + Copy`, `T: Ord + Display`, comparisons on `T` | basic generics OK |
| 21 | impl-trait | 2 | `impl Trait` in return position (argument position landed 8347eaf25) | return position unsupported |
| 21 | hashmap | 2 | `HashMap`/`BTreeMap`: insert/get/get_mut/entry/or_insert/or_default/remove/values/into_iter | std-paths first |
| 21 | dyn | 2 | `dyn Trait` objects, vtables, `Box<dyn T>` and `Rc<dyn T>` | "expected ASCII identifier, found \"dyn\"" |
| 21 | unsize | 2 | unsizing coercions `Box<S> -> Box<dyn T>`, `Rc<S> -> Rc<dyn T>`, `&Vec<T> -> &[T]` | needs dyn |
| 21 | box | 2 | `Box<T>`, `Box::new`, deref, recursive enums | "unresolved function" |
| 21 | or-pat | 2 | `a \| b` patterns (and inside `matches!`) | "or-patterns and @ patterns are unsupported" |
| 21 | ref-ops | 2 | arithmetic and comparison on references (`&i32 * &i32`), via std impls | unprobed |
| 21 | fmt-spec | 2 | format specs `{:.2}`, `{:02}`, `{:>8}` | unsupported placeholder |
| 31 | const | 1 | `const` items in types and expressions (`[[i64; N]; N]`) | "const items ... unsupported" |
| 31 | byte-lit | 1 | `b'a'` byte literals | "raw, byte and C literals are unsupported" |
| 31 | fn-traits | 1 | `F: Fn(i32) -> i32` bounds, passing `&closure` | "expected generic closing >" |
| 31 | prim-consts | 1 | `i32::MIN`, `i32::max` as fn value | "expected a local-root path" |
| 31 | ops-traits | 1 | `std::ops::{Add, Sub, Mul, Neg, Index}` user impls driving operators | std-paths first |
| 31 | rc-refcell | 1 | `Rc`, `Rc::strong_count`, `RefCell` borrow/borrow_mut | std-paths first |

### Original greedy unlock hypothesis

This order predates the first passing program and is not a current measured
schedule. Programs 01, 03 and 05 already pass. Prioritize reusable source-library
and trait machinery over isolated per-program special cases.

1. fmt-display + str: 01_hello
2. + for: 03_collatz
3. + string: 02_fizzbuzz
4. + array-iter: 05_gcd_fib
5. + fmt-debug + vec: 07_bank_account
6. + iter + match-ergonomics: 04_primes_sieve
7. + derive: 08_enum_state_machine
8. + closure + const + autoderef: 06_matrix
9. + prim-methods + byte-lit: 10_string_processing
10. + option-result + or-pat: 09_vec_stack
11. + box + ref-ops: 12_linked_list
12. + std-paths + hashmap: 17_hashmap_word_count
13. + user-fmt + ops-traits: 19_operator_overloading
14. + slice + try: 15_error_handling
15. + fmt-float + generic-bounds: 16_sorting
16. + format-macro: 18_tokenizer_calculator
17. + impl-trait: 14_generic_container
18. + fn-traits + prim-consts: 11_closures_iterators
19. + dyn + unsize + fmt-spec: 13_trait_objects
20. + rc-refcell: 20_inventory_system

The structural insight: everything past program 4 needs a real std library
surface (generic `Vec`/`Option`/`String`, traits, iterator adapters). That means
lang items plus a compiled library crate written in Rust, not per-method
special cases. `for` needs `IntoIterator`/`Iterator` plus `Range`, `fmt-display`
needs `Display` and `Formatter`, and derives expand to trait impls. Start with
library-hosted generic types and trait dispatch, and most tags fall together.

### Already working (probes pass with correct output)

Integer and f64 arithmetic, `as` casts, tuples and destructuring, arrays, nested
arrays `[[0i64; 3]; 3]`, type aliases, `&[T]` params with indexing, struct
methods (`&self`/`&mut self`/`Self`), unit, payload and struct-like enums,
tuple scrutinee matches, `use Enum::{A, B}`, generic structs and fns,
`loop`/`break value`, early `return`, `mut` params, `assert!`/`assert_eq!`,
`println!` with a literal-only format string.

Probe gaps worth noting for owners. A unit-struct value `S` used as an argument
(`g(&S)`) fails with "unresolved local S". An rvalue receiver `S.b()` fails with
"owned rvalue ... receivers are unsupported". `let a = [1, 2]; s(&a)` with
`s(a: &[i64; 2])` is rejected as `&[i32; 2]`, because integer literal inference
does not flow back from a later call argument.
