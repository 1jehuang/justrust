# Scalar literal and range patterns

Status: implemented and validated on 2026-10-18. Full Rust remains open.

Supported in `match`, `if let`, `while let`, `let else` and `matches!`, at the top level and nested inside
tuple, struct, enum and array patterns:

- character literal patterns (`'a'`, escapes and `\u{..}`),
- integer and character ranges: `a..=b`, `a..b`, `a..`, `..=b`, `..b`,
- integer endpoints as literals (with optional minus and suffix) or `iN::MIN` / `uN::MAX` paths.

Source ranges resolve against the exact scalar domain into one inclusive IR test,
`PatternTestKind::IntRange { lo, hi }` or `PatternTestKind::CharRange { lo, hi }`, plus
`PatternTestKind::Char`. Exclusive upper bounds subtract one (stepping over the
surrogate gap for `char`). Empty or reversed ranges (E0579/E0030), endpoint type
mismatches, out-of-range literals and negative unsigned bounds are type errors, even in unused
functions. IR validation re-checks endpoint canonicality, signed order and the
scalar domain for every decision row. CFG lowering is two ordinary `Ge`/`Le`
comparisons over the inspected place, so ownership, borrow and backend gain no new operations.

Usefulness and exhaustiveness use an exact interval partition: each split point
enumerates the elementary segments cut by every row's interval endpoints on
that path (value order, with signed integers flipped to unsigned ordinals) and skips
the uninhabited surrogate segment of `char`. 128-bit domains need no cardinality shortcut.

Not supported, with explicit diagnostics: constant/path endpoints other than
integer MIN/MAX, `X..` directly inside slice patterns (stock E0658), float
ranges, string literal patterns.

## Validation

`tests/scalar_patterns.rs` compiles 6 accepted programs (CHECK and BUILD at workers 1 and 8) and compares
their output with stock output recorded by `scripts/scalar_patterns_oracle.sh`.
It also requires native rejection of 17 stock-rejected programs. Unit
tests cover interval usefulness (surrogate gap, signed order, u128 extremes) and
public IR rejection of reversed, mistyped and non-scalar endpoints. Historical frozen
cases `matches_macro::{character,range}`, `guarded_matches::range_pattern` and
`match::range_pattern` became Live with unchanged sources and stock output.
