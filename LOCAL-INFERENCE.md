# Body-local structural inference

Status: local structural inference is implemented incrementally. This is not full
Rust inference or full closure support. GENERIC-INFERENCE.md describes the earlier
first-order generic-call foundation, not the current body-local solver's ceiling.

## Contract

Each function body's checker builds a private union-find table. Terms retain
canonical nominal declaration IDs and ordered generic arguments. Assignments,
call parameters and expected results, constructor fields, patterns and later
method uses contribute constraints before checked expression lowering. Integer
and floating variables default to i32 and f64 only after constraints settle.
Unresolved variables produce a type-annotations-needed diagnostic. Unused
invalid bodies remain checked.

Source-written expected types and solved hints have different authority. A hint
may solve a type, but must not create an implicit reborrow or change a required
unique-reference move. The ordinary checker still evaluates every expression
once, validates coercions, and produces the immutable checked body and closure
metadata consumed through the graph.

## Callable constraints

The executable callable increment covers scalar/unit signatures. Invoking a
reference-parameter callable still requires projected-method carrier admission.
Reference-bearing closure results, including nested carriers, receive an explicit
Unsupported diagnostic before adapter construction. These are distinct open
authority contracts, not unresolved inference or successful E0382 controls.
Original stronger reference-shape acceptance remains unignored in the private
pending reference-contract feature, rather than being reclassified as success.

A stored closure has a private callable term keyed by its canonical anonymous
path. Its parameters and result participate in body-local constraints from later
calls, expected results and canonical Fn/FnMut/FnOnce predicates. Lexical locals
use declaration-authenticated binding identities, including nested blocks and
shadowing. Callable terms do not become public type holes or fabricated closure
types.

Solved signatures are type hints for the authoritative parent-local closure
checker. That checker derives capture layout, actual receiver capability and
reference contracts from the real body. Inferred closure results do not grant
source-written return coercion authority. Published metadata retains the exact
owner, provider, site and mode-specific body proof. No new whole-program node,
shared mutable fact, or graph-integrity baseline is introduced.

## Acceptance

- tests/body_inference.rs covers later-use nominal constraints, nested expected
  types, patterns, constructors, loop binding modes and unresolved controls.
- tests/inference_move_authority.rs preserves moves through inferred locals and
  aggregates while retaining legal written-type reborrows.
- tests/generic_slice_inference.rs covers canonical shared/mutable structural
  slice inference and caller/callee binder substitution.
- tests/callable_inference.rs compares supported callable execution to stock Rust
  at workers 1 and 8 and rejects unresolved, arity, contradictory-type and
  unique-reference move controls.
- tests/callable_result_boundary.rs checks actual root and nested result carriers
  at workers 1 and 8 in CHECK and BUILD and verifies no image is published.
- graph_integrity remains mandatory. Broader iterator execution, direct generic
  closure arguments and reference-return capture admission require their own
  authoritative integration gates, not inference-only approximations.
