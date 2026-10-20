# Nominal recipe selectors

Status: focused correction implemented, 2026-10-20. This is not full Rust
support or promotion of the unchanged full-program corpus.

## Frozen contract

A nominal declaration recipe describes named or positional *fields*. An index
expression is not a field lookup, even when its index is a constant integer.
Indexing arrays/slices constrains the operand to `usize` and produces their
element type. Indexing a nominal value selects the canonical `Index` or
`IndexMut` trait with the actual operand type, and uses the selected reference
result's referent as the expression type. No field position is invented from an
index, and arbitrary custom index types remain distinct.

The ordinary executable checker still independently validates trait selection,
reference authority, initialized storage, mutability, bounds, moves and loans.
Inference results are not executable capabilities. Unresolved operator
constraints block nominal preparation instead of publishing a solved manifest.
Unsupported inputs continue to reject, without placeholder success.

## Root cause and representation

`local_literals::project` previously dispatched every selector on a nominal to
`prepare_field`. Expression indexing produced `ConstantIndex`, which the
recipe consumer correctly rejected as `non-field selector in nominal recipe`.
The fix restricts this dispatch to `Named`/`Tuple` and represents indexing with
an explicit local operator constraint, retaining receiver, actual index term,
mutability selection and result term. Static place indexes take the same path.
References are peeled when solving, including receivers learned after queuing.
Explicit `Deref` projections are different: they must consume exactly one
reference/pointer layer. Previously, `project` removed reference layers before
looking for `Deref`, leaving reference-place updates permanently unresolved.
The corrected dispatch retains that layer and reads its referent. It does not
turn a nominal `Deref` selector into a field recipe or grant mutable authority.

The consumer's non-field, field-range and argument-arity rejections are retained.
No graph node kind, global state, mutable fact carrier, authority bypass,
whole-program join, budget or integrity ratchet is added. The local constraint
is consumed inside the existing body graph operation. Final lowering and CFG
shape remain the existing checked index operations and canonical calls.

## Acceptance gates

- `tests/nominal_recipe_selectors.rs`: stock/native CHECK, BUILD and execution
  at workers 1 and 8 for constant/dynamic Vec indexing, mutable Vec indexing,
  builtin arrays and custom `Index<bool>`. Includes invalid builtin index control.
- `tests/nominal_recipe_selectors_authority.rs`: invalid nominal without an
  implementation, wrong operand type, immutable assignment and invalid unused
  function reject at both roots/workers. Shared index borrows cannot authorize
  writes through an explicit dereference. A valid control guards against blanket
  rejection.
- `nominal_manifest_tests::nominal_recipe_selectors_reject_non_fields_and_out_of_range_fields`:
  directly forged constant-index/deref selectors, out-of-range field positions
  and incorrect actual arity must reject without emitting executable expressions.
- `tests/nominal_recipe_selectors_graph.rs`: nominal indexing CHECK reads
  declaration facts, publishes no executable and demands no codegen/optimizer.
- `justrust test --test graph_integrity`: mandatory unchanged structural gates.

## Observed receipts and remaining boundaries

Hosted run `20261020-025823428-3575045` passed 22 tests: all 14 graph-integrity
checks, the first four selector acceptance/corpus tests and four negative
controls. Hosted run `20261020-030141503-3580598` additionally passed the custom
`Index<bool>` stock/native CHECK/BUILD/run test at workers 1 and 8.
After the coordinated lint cleanup, hosted run `20261020-030530712-3590263`
passed the direct forged-selector unit test, and
`20261020-030542342-3590607` passed the graph-specific nominal indexing test
at workers 1 and 8.

The unchanged 04 primes sieve, 10 string processing and 18 tokenizer/calculator
fixtures were CHECKed at workers 1 and 8 after the initial index correction.
All advanced beyond the internal
selector rejection to:

```text
unsupported: nominal source constraints cannot progress from the exact available graph inputs (source schemas needed: false, unresolved constraints: true)
```

They are **not** claimed to compile or run. Their next inference/integration
boundaries remain open. The later explicit-reference correction does not promote
the corpus without a fresh unchanged-program run. No performance claim is made.

### Explicit reference projection follow-through

The supplied `c5d75a98e` source archive established that the unchanged
`vec_mutable_index_places` already failed with the original non-field diagnostic
(hosted run `20261020-031249918-3614661`), while
`vec_index_format_borrows_noncopy_once` already failed with unresolved nominal
constraints (`20261020-031300484-3615014`). These were not newly introduced
failures. After the initial index fix, the mutable case reached that same
unresolved-constraint boundary. Keeping the reference layer for explicit
`Deref` then made both unchanged tests pass with their existing stock/native
CHECK/BUILD/run comparisons at workers 1 and 8:

- `vec_mutable_index_places`: `20261020-031257735-3614899`.
- `vec_index_format_borrows_noncopy_once`: `20261020-031355043-3614898`.

Final hosted validation of the complete correction passed the shared-index-borrow
mutation rejection control (`20261020-031505885-3618500`) and all 14 unchanged
graph-integrity gates plus the nominal-selector graph test
(`20261020-031608446-3621575`, 15 tests). No integrity baseline was changed.

The broader `deref_index_traits` suite separately exposed an existing negative
control expecting an `E0277` code where rejection currently reports only
`trait obligation is not proven by caller bounds or a conditional implementation`.
Its assertion was not weakened. This diagnostic follow-up is distinct from
indexing correctness.
