# Scalar `{}` formatting in print macros

Status: stock corpus frozen, native implementation in progress (not merged).
Full Rust remains open.

## Intended slice

`println!`, `print!`, `eprintln!`, `eprint!` with bare `{}` placeholders over
integer (all widths, including i128/u128 extremes), `bool` and `char`
arguments. Arguments are evaluated once, left to right; `{{`/`}}` escapes are
preserved; argument-count mismatches and non-Display types are rejected, even
in unused functions.

Explicit unsupported boundaries: `{:?}`, width/precision, positional `{0}`,
inline named `{x}`, floats, `&str` arguments, reference arguments, and
primitive associated constants such as `i8::MIN` (the last is an existing
path-resolution gap, so the integer extremes case uses typed literals).

## Corpus

`tests/fixtures/format_scalars/stock.json` records stock rustc 1.98.1
(`--edition=2024 -C panic=abort`) results for 20 sources: 7 positive,
5 invalid, 8 legal-unsupported boundaries. It was produced before any native
expectation.

## Resume point

Work-in-progress compiler changes (AST `Expr::Format`, bounded `{}` splitter,
argument parser helper) live outside the repository as a patch. Remaining:
wire parser dispatch, type checking, IR/CFG node with operands, ownership and
borrow visitors, native decimal emission (including 128-bit), an acceptance
harness, and integrated regression.
