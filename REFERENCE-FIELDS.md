# Builtin reference field access

Status: implemented. All 942 hosted tests, strict all-target lint, formatting
and both independent actual-code reviews pass. The frozen 20-case stock/native
CLI corpus and four retained graph tests pass. Tracked nineteen-corpus native
verification follows. This is one Rust correctness slice, not full Rust.

## One real dereference, not a field-type shortcut

For builtin shared/mutable references, source `r.field` and `r.0` normalize to
`(*r).field` and `(*r).0`. Insert one actual checked IR Deref or Place Deref
projection before the selected field. Do not merely peel the field lookup type
while leaving the place rooted in the reference carrier slot. Nested reference
types remain unsupported, so this slice needs no repeated autoderef or trait
resolution. Explicit `(*r).field` is already normalized and must not gain another
Deref. Ordinary nested fields and static array indices compose after that Deref.

Read expression typing evaluates its receiver once and preserves completion and
source order. Place creation for assignment and borrowing preserves the original
carrier rather than moving or snapshotting its non-Copy referent. Scalar hints
perform the same type normalization without evaluation. Existing RHS snapshot
ordering remains authoritative for assignment, so an RHS that retargets a mutable
reference makes the final write resolve the current carrier after RHS evaluation.

## Authority and visibility

Field lookup and accessibility checks run after peeling the Ref and retain
nominal field privacy. Shared references cannot authorize writes or mutable
borrows, even when their carrier binding is mutable. An immutable binding holding
an &mut reference may modify its referent. Copy field reads do not move the
reference carrier or its parent aggregate. Non-Copy moves through any Deref remain
invalid. Loans, provenance, initialization, selection proofs and lifetime checks
continue to run on the same exact normalized CFG and canonical target paths.

A genuine reference Call result may supply a field READ through its existing real
result temporary, with only final Copy-pointee extraction admitted. There is no
computed Call LHS or borrowing of a Call rvalue. Retain the current local/call
receiver restriction: complex reference-valued block/if receivers can remain
explicit unsupported rather than pretending they are static places.

## Unchanged boundaries

No new public IR/CFG operation, type, provenance contract, graph fact or native ABI
is needed. Production changes belong only in frontend.rs. Every existing guard
that detects Deref from source AST must recognize inserted implicit Deref too.
In particular runtime indexing through references and enum downcasts/matching
through Deref remain unsupported. Static field/ConstantIndex composition stays
within the existing single-Deref path and depth limits. Bare reference indexing,
methods, trait-based Deref, autoref, nested reference types, mixed-kind reference
result contracts and reference-containing aggregates are not enabled.

## Acceptance and delivery

Use the frozen stock20 corpus with ten accepted programs, six invalid programs
and four legal-unsupported boundaries, SHA256
`01a34702d4e36424cc5d0ce35d900ee4ad00a2b6af174b3589fedeb2877b4f8b`.
Cover struct/tuple reads, mutable field writes, explicit/implicit equivalence,
nested static paths, field borrows and genuine call-result reads. Reject shared
writes, private field access, non-Copy moves and lifetime violations. Retain graph
facts, all-body failure publication and actual native original-storage behavior.

Historical implicit projected getters may be promoted only after actual native
acceptance, with every existing source/stdout/value byte preserved. Audit all
historical corpora for the same normalized capability, not only two known cases.
No unsupported computed assignment, dynamic reference index or method fixture may
be reclassified to hide a boundary regression. Final gates are full hosted tests,
strict all-target lint, formatting, independent actual-code reviews, a tracked
nineteen-corpus native run and retention of raw receipts and the actual example.
Full Rust, including general autoderef, remains open. No speedup is claimed.
