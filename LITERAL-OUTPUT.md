# Literal stdout/stderr macros

Status: implementation in progress, not delivered. Full Rust remains open.

Add checked builtin `print!`, `eprint!`, and `eprintln!` for a single string literal with optional trailing comma and all three delimiters. Empty `eprintln!()` emits a newline. Empty print/eprint are rejected consistently with stock. Reuse literal escape/escaped-brace decoding; reject interpolation, extra formatting arguments and nonliteral inputs explicitly. Builtins are selected only after authoritative lexical/module macro lookup misses, with checked invocation identity and source ownership. Existing `println!` behavior and its historical source fixtures stay unchanged in this slice.

Freeze shared native payload as `ir::LiteralOutput { text: String, stream: ir::OutputStream, newline: bool }`, with `OutputStream::{Stdout,Stderr}`. Source is `SourceOutput { output: ir::LiteralOutput, site: SpanId }` and `Expr::Output(Box<SourceOutput>)`; IR is `ExprKind::Output(LiteralOutput)` and CFG is `Rvalue::Output(LiteralOutput)`. Output is exactly Unit/Returns, has no operands, and must be validated in unused rows too. All payload storage and cloning must use existing budgets; no arbitrary file descriptors or forgeable stream integers.

The Linux x86-64 backend reuses the existing literal write loop with fixed descriptor 1 or 2, optional trailing newline, partial-write/EINTR handling and the existing terminal error policy. Empty no-newline output returns Unit without a zero-byte syscall/trap. No new buffering, flushing, broken-pipe parity, formatting traits or platform claims. Preserve output order within each stream. Stock/native test commands disable core dumps where needed.

Acceptance uses independently measured frozen source text, exact stdout and stderr separately, workers1/8 CHECK/BUILD in CLI and graph routes, debug/release, shadowing/delimiters/empty/Unicode/escaped braces/unused and late-invalid controls, and checked source/IR authority. Full regressions, formatting and strict lint precede publication. No speed or full formatting support claim.
