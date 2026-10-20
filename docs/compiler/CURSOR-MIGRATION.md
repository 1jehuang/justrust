# Immutable parser cursor checkpoint

This is the T0a prerequisite in `MACRO-EXPANSION.md`, not macro expansion,
a new Rust lexer, hygienic name resolution or full Rust support.

## Production change

The existing lexer still produces the same owned tokens and exact prepared-byte
ranges. The parser borrows that immutable input through a private `TokenCursor`.
A by-value `TokenView` borrows spelling from the input, not from mutable cursor
state. The AST continues owning semantic strings as before.

Only type parsing splits `&&`, `>>`, `>=` and `>>=`. Splits advance an ASCII
prefix offset in the cursor and preserve the original token spelling and range.
Expression consumption retains whole lexer operators. Quoted punctuation cannot
split, and an empty string literal is distinct from the final EOF token.

Checkpoint restoration validates the original input slice identity, including
its length, the token index and the permitted punctuation-prefix offset. A
foreign or invalid mark leaves the cursor unchanged. EOF consumption is stable.
The existing array-count syntax fallback restores a cursor checkpoint rather
than a bare token index. It does not roll back resource counters or introduce a
macro-fragment matching fallback.

All parser token accesses, lookahead and advancement go through the cursor.
There is no second authoritative lexer and no printing/relexing path. Prepared
source mapping, raw inactive syntax preparation boundaries, AST publication,
constness, canonical source replay and ownership checks remain unchanged.

## Validation requirements

The checkpoint must retain the complete hosted compiler suite, strict all-target
lint and format checks. Focused tests exercise each type split and restoration,
foreign and invalid marks, repeated EOF, quoted punctuation, untouched expression
operators, nested reference/type parsing and array-count diagnostic categories
and spans. Existing Unicode/CRLF source mapping, configured-file graph checks,
source-locked identity and nested-reference acceptance remain regression gates.

The standalone session-qualified token/tree/span/hygiene and expansion-budget
modules are a separate subsequent lane. Their unit tests cannot establish
native macro acceptance. Real generated-token parsing, hygienic AST identifiers,
matching/transcription, ordered discovery and CHECK/BUILD graph completion remain
open.

## Frozen acceptance, 2026-10-17

- All-target hosted check: `20261017-060334104-402872`.
- Complete hosted suite: `20261017-060335863-403027`, 1766 tests in 93 binaries,
  no failures or filtered tests.
- Strict all-target lint: `20261017-060350663-404097`.
- Full frontend filter: `20261017-060302085-400443`, 179 tests, including exact
  Unicode/CRLF split coordinates and held views across cursor changes.
- Source units: `20261017-060049783-390487`, 61 tests.
- Source configuration, identity and nested-reference integration/graph targets:
  `20261017-060052585-390594`, 19 tests across the six unchanged binaries.

`cargo fmt -- --check` and `git diff --check` pass. Independent read-only review
found no concrete blocker in the bounded cursor change. Six exact redacted run
summaries are retained in `data/runs/xps14/2026-10-17-cursor.jsonl`.

The full suite's slowest wide-integer stock/native conformance test took about
12.2 seconds. These are correctness gates, not a before/after performance
experiment, and no speedup is claimed.
