# Scoped local numeric literal constraints

Local unsuffixed integer and float literals can now receive type evidence from
later uses before the ordinary expression checker applies i32/f64 fallback.
This is a scoped scalar increment, not general Rust type inference.

## Mechanism and integrity

`frontend/local_literals.rs` collects lexical equality constraints in local
scratch inside the current body-check graph computation. Union-find classes
retain unknown, integer, float, or rigid numeric domains. Existing parameters,
annotations, suffixed literals and nongeneric call signatures are rigid.
Conflicting integer widths, float widths or literal classes are rejected.
There are no new graph kinds, shared state, result slots, external compiler
inputs, or integrity-baseline changes.

Supported evidence paths include scalar let aliases, arithmetic and comparison
peers, assignments and compound assignments, explicit returns and expected block
tails, nongeneric function arguments/results, nested blocks/if/while, and simple
range-for bindings. Lexical declaration/reference keys preserve hygiene, old
initializer frontiers and shadowing. Shift operand widths remain independent.

Only resolved numeric classes supply initializer expectations. An unresolved
integer/float class supplies no hint, so the existing checker can still obtain
stronger field/generic-call peer evidence before its normal fallback. Every
initializer, literal range, operation, call, return, assignment and generated IR
still passes ordinary checking. A hint never constitutes acceptance or fallback
from an error.

## Acceptance

`tests/local_literals.rs` has 28 stock-differential sources, including the exact
unchanged `03_collatz.rs` and its frozen stdout, later returns/assignments,
transitive aliases, nested uses, integer/float defaulting, explicit/suffixed
rigidity, conflicting constraints, integer range/sign errors, and field/generic
peer regression controls. The stock oracle is external only. Run locally when
hosted workers do not provide stock rustc.

The full-program corpus promotes `03_collatz` only after native check/build/run
and byte-for-byte stdout comparison. It prints:

```text
longest chain under 10000 starts at 6171 with 261 steps
collatz(27) = 111
```

Final validation on composed `ae799cafd` plus this change:

- `cargo fmt --check` and hosted `justrust check`: pass
  (`20261018-193224598-443343`).
- Hosted `justrust test --lib --test graph_integrity --test loops --test compiler
  --test full_programs --test local_literals`: 1,404 tests pass, including all
  five unchanged integrity gates (`20261018-193232382-449668`).
- Hosted `justrust clippy --all-targets -- -D warnings`: warning-free
  (`20261018-193253292-439049`).
- Local `justrust test --local --test local_literals --test format_scalars`:
  all three tests pass, with real stock rustc for all 28 inference sources and
  the existing formatting oracle (`20261018-193156673-419201`). Local execution
  is required because hosted workers lack the stock oracle executable.

Only these four task-owned exported receipts were appended to `data/runs/`.
These are correctness gates under contention, not speed measurements.

## Boundaries

This does not add general structured inference, future-use reference inference,
method/generic-call constraint solving, match-pattern inference, loop-break
inference, or new iterator forms. Unsupported expression forms contribute no
speculative evidence and remain the ordinary checker's responsibility. A valid
Rust program depending on an unimplemented evidence path can still be rejected.
Full Rust support and the remainder of the corpus remain open. No speed claim is
made.
