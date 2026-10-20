# Scoped immediate shared capture acceptance

These are redacted stable-client receipts, not full-Rust or speed evidence.
`../../../docs/compiler/CLOSURES.md` defines the admitted immediate, explicit-signature, shared scalar
parent-parameter subset. Stored/local/mutable/move captures and callable traits
remain open. No graph-integrity ceiling or mutable-carrier baseline changed.

## Capture implementation before current-origin repair merge

- `20261018-203203477-2641142`: local `--test closures`, six tests passed.
  The two stock/native oracle tests compare 25 complete programs (14 existing,
  11 captured), including unused-body type/move negatives and argument writes
  and mutable borrows conflicting with the environment's shared loans.
  Other tests check source identity, nesting, unsupported boundaries and equal
  owner/site fact keys across workers 1/8 and check/build.
- `20261018-203205350-2642140`: hosted graph-integrity seven tests passed.
- `20261018-203218118-2640799`: hosted all-target clippy, `-D warnings`, passed.

## Composed repaired snapshot `3373825ef`

Capture checkpoint `c7d042ea0` was rebased on actual origin `a265b4922`.
The composed panic suite rejected ordinary nonclosure cells with
`owning Drop CFG requires exact canonical lowering proof`. Coordinator confirmed
this exact origin regression and provided repair `94482d101`, preserved through
non-destructive merge of `942b537dc5efee16dc14e7155b14b7bbb1ac8531`.
No fixture source or expectation was changed to accommodate that failure.

- `20261018-203532366-2707332`: hosted `check`, passed.
- `20261018-203543476-2712481`: local tests `closures`, `panic`,
  `generic_inference`, `wide_integers_graph`, `literal_output_authority`,
  **50 passed**, including the previously failing exact panic suite.
- `20261018-203620956-2727969`: hosted graph-integrity **seven passed**.
- `20261018-203634922-2705188`: hosted all-target clippy `-D warnings`, passed.
- `cargo fmt --check` passed in the same command sequence.

Local execution is required for the stock-rustc oracle and its actual executable
programs. Hosted checks, graph gates and lints use the installed stable client.
No experimental compiler was installed and the main worktree was not built or
edited. A full suite was not rerun for this capture increment. The earlier full
suite and its exact repair boundaries remain documented in
`data/runs/closure-validation/README.md`.
