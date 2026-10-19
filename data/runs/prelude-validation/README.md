# Source prelude validation

These four records are unchanged slim, path-redacted summaries selected from
`justrust export` on xps14 for the source prelude increment. Other machine
history is intentionally not duplicated here.

- `20261018-193737590-829183`: hosted `justrust check` passed.
- `20261018-193745015-838508`: hosted all-target clippy with `-D warnings` passed.
- `20261018-193748290-816885`: 110 tests passed across 11 prelude, methods,
  integrity, exact source/callable graph, compiler, module, tuple and loop targets.
- `20261018-193803506-863111`: configured source graph target, 4 tests passed.

The separate local affected-corpus run encountered existing panic/guarded-match
boundary expectation drift after scalar formatting became supported. Corpus
owners are repairing those with frozen-source stock/native evidence. This is
not a claim that the complete Rust test suite passed.

The integrity work-growth ceiling tightened from 1.92 to 1.81 because six fixed
library universal proofs shift the measured 16-to-32-function work ratio to
1.757. This is not an algorithmic improvement or a compile-time speed claim.
The leaf-closure and required-program-size gates were not loosened.
