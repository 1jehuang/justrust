# Primitive methods

Recovered implementation from `4b0e5b624`, with the preserved receiver-autoderef
checkpoint. This is a supported subset, not full standard-library support.

The closed scalar/signature table in `src/compiler/ir/intrinsics.rs` owns method
selection and types. Typed intrinsic expressions are checked by existing graph
outputs and source-authority replay. CFG expansion uses existing typed arithmetic,
comparisons and branches. Only float sqrt, abs and sign-bit queries need native
backend kernels. No graph node kinds, side state or ratchets are added.

The body-local solver also consumes this exact signature table (2026-10-20),
including primitive method arguments and unannotated results, instead of trying
ordinary trait selection alone. Fixed receiver types are required; this does not
infer a scalar receiver from a method's name or expected result. The separate
leading-zero operation now shares its exact signature between inference and
checked lowering. `LEADING-ZEROS.md` records the hosted ten-source stock
CHECK/BUILD/run matrix at workers 1/8, including ordinary same-name trait methods.
The new matrix uses the verified pinned stock-rustc helper on hosted builds;
the older full-suite local-oracle instructions below remain historical.

## Source-backed decimal parsing

`str::parse` dispatches through ordinary `std::str::FromStr` / `core::str::FromStr`
implementations. The source prelude implements all twelve integer types, including
full-range `i128` and `u128`, and exposes `ParseIntError::kind`, `IntErrorKind`,
Display and Debug. Explicit arguments, contextual `Result` and chained `unwrap`
inference use canonical callable schemas, not a parse-specific intrinsic.
User implementations and generic callers follow the same path. Owned method
receiver temporaries keep full-expression drop order, and source-written static
error lifetimes cannot borrow a parser input.

`justrust test --test primitive_methods integer_from_str` compares stock CHECK,
BUILD and execution at workers 1/8, including every integer boundary, signs,
invalid syntax, custom implementations, drop order and unused invalid bodies.
Floating-point parsing and full standard-library compatibility remain open.

Source-backed `FromStr` also covers `bool` and `char`, with public
`str::ParseBoolError` and `char::ParseCharError` exports through std and core.
Booleans accept only exact lowercase `true` and `false`. Character parsing
accepts exactly one Unicode scalar, not one byte or one grapheme cluster.
`justrust test --test primitive_methods bool_and_char_from_str_match_stock`
checks direct and generic dispatch, owned strings, error Display/Debug,
Unicode and invalid result types against stock at workers 1/8.
Contextual inference through a generic free call followed by `unwrap` and
some string-reference equality lowering remain separate open compiler gaps.

Supported methods:

- Integers: signed abs/signum/is_positive/is_negative, pow with u32 exponent,
  min/max, wrapping_add/sub/mul, signed wrapping_neg, saturating_add/sub,
  count_ones/count_zeros/trailing_zeros, unsigned is_power_of_two.
- Characters: ASCII classification and ASCII case mapping, is_whitespace and
  is_control. Unicode whitespace/control sets are explicit scalar ranges.
- f32/f64: sqrt, abs, min/max, is_nan/is_finite/is_infinite,
  is_sign_positive/is_sign_negative. A NaN on either side of min/max yields
  the other operand. Both NaN yields NaN. Equal signed zeros may select either.

Current boundaries: wrapping/saturating methods reject 128-bit receivers with an
explicit unsupported diagnostic. Receiver autoderef is bounded at eight layers.
Other primitive methods and general standard-library method resolution remain
outside this increment. Integer overflow follows the compiler's existing checked
arithmetic path, not an independently implemented release-overflow mode.

`justrust test --test primitive_methods` compares execution/exit status and
invalid-input rejection using edition 2024 and panic=abort. The integer parsing
cases use the verified stock-rustc helper, including in hosted test environments.
Cases cover all supported operations, reference receivers, wide integers, bit
boundaries, Unicode/ASCII distinctions, NaN operand ordering, and receiver/argument
evaluation order. No speed or full Rust compatibility claim is made.
