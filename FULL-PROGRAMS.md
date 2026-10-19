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

## Status (2026-10-19, origin/clean-room-rewrite 6ae2c3e)

Landed since bf4ad37: char literal/range patterns (671f9090f), argument-position
`impl Trait` (8347eaf25), rvalue method receivers (2dcaa3841), char casts.

3/20 pass: `01_hello`, unchanged `03_collatz`, and unchanged `05_gcd_fib`
check, build and execute with exact frozen stock stdout. The numeric programs
combine native scalar formatting, loops and scoped future-use numeric literal
constraints, including bidirectional range/array item evidence
(`LOCAL-LITERALS.md`). Other programs retain explicit rejection diagnostics
until their remaining features land. This is not full formatting or full Rust
support. See `FORMAT-SCALARS.md`.

## Ranked blockers (feature -> programs blocked)

The count is how many programs need the feature, from a manual read of each program.
First-diagnostic counts undercount, because every program stops at its first
error. "Probe" is the native diagnostic for a minimal program using only
that feature (values observed via `if cond { println!("ok") }`).

| # | tag | blocks | feature | probe today |
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

### Greedy unlock order (fewest new features per program)

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
