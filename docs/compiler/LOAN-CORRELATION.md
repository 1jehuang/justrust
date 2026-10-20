# Conditional loan-origin correlation

Status: implemented and stock-differentially validated for explicit loan pairs.
General lifetime and arbitrary returned-carrier correlation remain open.

## Frozen contract

A joined carrier origin set represents alternatives, not simultaneous loans.
A borrow conflict may be excluded only by an immutable, complete per-function
may-coexist witness proving that a required authority origin of the selected
access target cannot coexist with the conflicting origin on any reaching path.
No origin or target is removed from ordinary carrier, reborrow, move, return,
or loan validation. Unknown transport and incomplete evidence remain conservative.

## Analysis

For each candidate conflict involving two explicit loan identities, compute a
forward finite fixed point over existing CFG blocks. The immutable domain is a
set of four possible worlds: neither origin, first only, second only, or both.
Joins union worlds and never form a cross product between alternative predecessor
states. Explicit loan issuance sets that origin's bit in every reaching world.
Existing solved point liveness and transitive lifetime dependencies conservatively
prune dead origins. Parameters, returned carriers, missing point evidence and
non-unique static issuance sites cannot use the exclusion proof. Copies preserve
origin identities rather than generating new loans. Static loan renewal remains
independently gated by an all-path kill-before-rebirth proof.

The witness is built inside the existing per-callable borrow producer, uses only
existing per-task charged work/storage APIs, and is immutable after construction.
No lazy cache, shared budget, new whole-program operation, or global state.

## Acceptance

Stock-positive conditional replacement of a mutable reference must remain visible
and pass native CHECK/BUILD/run at workers 1 and 8. The feature driver applies
that matrix to all six stock-positive cases, not only the replacement fixture.
Test fresh branch-local loans,
transitive reborrows, early exits and loop backedges. Stock-invalid simultaneous
aliases, cross-branch escaping aliases, dangling references and missing kills must
still receive genuine invalid diagnostics, never success or Unsupported alone.
Run existing borrow authority tests, graph_integrity and all-target no-run.
Keep renewal and correlation patches independently attributable.
