# Mixed-capability reference-result inputs

Status: implemented. All 975 hosted tests, strict all-target lint and formatting
pass. Exact stock20/native CLI20 and four graph tests pass, with actual-code
review clear. Tracked twenty-corpus native verification follows before delivery.
Builtin reference fields are delivered with 942 hosted tests and 732 independently
verified native cases, implementation a0f8ba730 and evidence bcc845ce9. This next
slice does not complete Rust references or full Rust.

## Capability eligibility, not input-kind equality

Keep RefContract, InputPath, SharedInputPaths and MutInputPaths unchanged. Remove
only the blanket requirement that every reference input match the result kind.
Shared results may derive complete type-compatible static input paths from shared
OR mutable inputs. Mutable results may derive paths ONLY from mutable inputs.
Unrelated reference inputs may be either kind. A shared input can never supply
mutable authority, even if regions coincide or its layout matches another input.

The declaration-derived candidate list stays complete, sorted by original
parameter index then lexicographic path. Retain exact nominal/structural referent
equality, declared outlives filtering, private internal paths, opaque enums and
Field/ConstantIndex-only suffixes. Do not infer a smaller body-derived summary.
Validator recomputation must reject forged shared-to-mutable candidates, missing
or extra paths, kind mismatches and malformed metadata even in dead CFG.

## Preserve genuine capability conversions

Frontend region normalization considers EVERY reference input regardless of kind.
Output elision still requires exactly one input reference lifetime POSITION, not
one distinct region ID or one eligible candidate. Explicit outputs remain rigid,
with no invented anonymous-to-named bounds. Named plus synthetic regions stay
bounded by 64 before growth. NoReference retains erased B2 full-arity None metadata
and its existing large-signature behavior.

A direct mutable carrier at an expected shared return site uses the existing real
Shared Borrow of carrier.Deref BEFORE reading or moving that carrier. Explicit
&*r and projected shared borrows remain genuine shared loan sites. Do not relabel
mutable formal origins shared, reinterpret a mutable handle as a shared copy, or
move the carrier first and fabricate a surviving parent. Shared-to-mutable remains
invalid. Explicit &mut borrow expressions coercing to shared expected results
remain the existing legal-unsupported boundary in this slice.

## Authority, lifetime dependencies and mutation

Formal origins retain declared input kinds. CallReturn origins derive the actual
result kind. Construct every result target from its own candidate.param actual
origin plus that candidate suffix, retaining only its own ancestry. Never merge
authority exemptions for equal addresses or union unrelated input permissions.

Lifetime dependencies still include ALL actual inputs whose declared regions
outlive the output, independent of kind, pointee type or candidate eligibility.
Freeze actual origins before moved argument temporaries are consumed. Preserve
transitive dependencies across nested and recursive returned calls. A mutable
noncandidate input may constrain the output lifetime without granting it target
authority. Distinct independent regions can end after the call when unrelated.

All argument loans remain live through invocation and later argument evaluation.
Overlapping mutable/shared or mutable/mutable actuals reject, shared aliases may
coexist. Every mutable actual still invalidates all possible referent proof
subtrees on return, including noncandidates. Result writes require every possible
target initialized, preserve initialization bits and kill all possible target
proofs, never initialize every candidate as if all had been written.

Shared normal returns retain Copy behavior. Mutable normal returns prove exact
provenance and universal dependencies at Read phase BEFORE unique Move. Parent
freeze/resume, child reborrows, current-carrier RHS ordering and all-body publication
gates remain unchanged. No native ABI or public IR/CFG opcode change is needed.

## Bounds, scope and acceptance

Retain 64 regions, 4096 combined explicit/formal/CallReturn origins, 4096 candidate
paths, 131072 stored steps and occurrence visits, composed depth128, one million
memberships, shared128MiB state and32M work. Count newly eligible mutable-input
paths with the same before-allocation and shared metering rules. No partial lists,
truncation, unchecked pointer path or inferred recursive body summary.

Freeze exact stock20 corpus SHA256
`0cc633f928bdf504c092c6124eafa4f4edb20b200575c59470bf71e8466497ae`:
12 accepted, six invalid, two legal-unsupported coercion cases. Require stock
classification, native workers1/8 check/build/runtime, retained graph facts and
all-body failure publication, malformed capability metadata and lifetime/proof
checks, full hosted suite, strict all-target lint, formatting and independent
actual-code review. Finish with tracked twenty-corpus native verification and raw
receipts plus an executed high-width original-storage example. Historical cases
may be promoted only after actual native acceptance with source/output unchanged.
Nested references, reference aggregates, general coercions, methods, trait Deref,
unsafe, Drop, standard-library projects and all other full Rust gaps remain open.
No speedup is claimed by correctness gates.
