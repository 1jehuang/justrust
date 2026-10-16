# Enums and match: first native milestone

This records the original fieldless milestone. `PAYLOAD-ENUMS.md` extends it
with tuple/braced variants, typed projections and selected-variant proofs.
Statements about payloads being deferred below describe this historical slice,
not the current combined supported boundary.

This is an implementation slice toward full Rust, not a full enum or pattern
implementation. Payload variants follow this slice with sum-aware move paths.
The implementation must not delegate compilation to another compiler.

## Source contract

Support nonempty nominal enums with unit variants, including visibility, qualified
variant paths and simple imports/aliases. Variants have declaration-order internal
tags. Explicit discriminants, payloads, derives, repr, generics and empty enums are
explicitly unsupported in this slice, not invalid Rust. Bound variants to 4096.
An enum is non-Copy even though its current native representation is one word.

Support expression `match` over enums, bool, unit and i32, with variant/literal,
wildcard and whole-value binding arms. Bindings may be mutable. Each arm has a
fresh lexical scope and the initializer/scrutinee uses the outer scope. Enum,
bool and unit coverage must be exhaustive. i32 literal matches require a catchall
until integer-range coverage is implemented. Guards, or/range patterns and nested
refutable payload patterns remain unsupported. Every arm is type checked,
including unreachable arms. Unreachable patterns are not errors merely because
stock Rust warns about them. Preserve semantic type independently of completion.

## Immutable semantic facts

`EnumTy` is an immutable Arc handle with identity-only equality/hash using DefId,
just like StructTy. `EnumDef` contains identity, canonical name, defining module,
visibility and declaration-order variant names. Debug output is shallow/bounded.
Snapshot-local IDs are not persistent cache keys. Struct and enum declarations
must have disjoint identities within the shared declaration environment.

Checked IR adds `Ty::Enum`, `ExprKind::EnumVariant { ty, variant }`, and
`ExprKind::Match { scrutinee, arms }`. Checked match patterns contain only resolved
variants, scalar values, wildcard or a unique LocalId binding. Enum variant
indices and owners must be checked, not trusted as arbitrary tags.

## Place semantics and control flow

Evaluate the scrutinee once. Address a local/projected place without consuming it;
materialize a non-place expression once. A variant test reads its discriminant
without moving/copying the enum. A selected binding consumes the whole projected
value via existing Copy/Move rules. A wildcard does not read or consume it.
In particular, wildcard-only matching a moved place remains legal, while testing
variants of a moved fieldless enum fails initialization analysis. Repeated variant
matches do not move the enum. Rvalue effects still execute for wildcard matches.
Do not emit an unconditional discriminant read before an irrefutable first arm.
A singleton enum's sole variant is irrefutable and requires no tag read, even
on a moved place. Nevertheless, later unreachable arms still participate in
static ownership checks and post-match initialization joins. Lower an irrefutable
choice with later arms to a constant-true boolean switch: execution takes only
the selected arm, while analysis visits both successors, including any later
constructor tag reads. A wildcard-only match therefore differs from a wildcard
followed by an unreachable variant test, exactly as in the stock oracle.

Lower arms to existing explicit CFG boolean switches, ordered tests and joins,
with branch-specific binding storage lifetimes and independent result snapshots.
Add `Constant::Enum { ty, variant }` and `Rvalue::Discriminant(Place)` producing
I32. The analyzer treats discriminant as a nonconsuming initialized-place read,
not as `Copy` permission for a non-Copy enum. A fieldless enum is a leaf move path.
The backend uses the existing scalar-word calling convention for this fieldless
representation, and raw tag loads, never manufactured semantic Copy operations.

## Shared graph and validation

No new crate/phase barrier or duplicate analysis: declaration fact 101 prepares
one immutable environment; BODY 102, LOWER 108 and ANALYZE 109 are each cached once
per function. Both check and native emission require the same analysis fact.
Expose enum counts alongside existing source/function/struct/CFG/analysis metrics.

Validate bounded native execution and rejected programs at workers 1 and 8,
independent stock classifications, no-read wildcard cases, selected binding moves,
match-result snapshots, branch joins, old-scope shadowing, privacy/imports,
exhaustiveness, dead-arm typing and malformed checked IR. Run hosted check/test/
strict clippy and a tracked M8a differential workload. Record full receipts.
No subset timing, cold-build or full-Rust claim follows from this milestone.
