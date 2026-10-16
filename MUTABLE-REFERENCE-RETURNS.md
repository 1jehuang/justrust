# B3b design checkpoint: mutable whole-parameter reference returns

Status: implemented with 844 hosted tests, strict all-target lint, formatting
and independent soundness/resource reviews passing. The exact frozen 30-case
stock corpus and four retained graph acceptance tests pass: 16 supported, ten
invalid and four legal-unsupported programs. Tracked sixteen-corpus native
verification remains the delivery gate. Full Rust, projected reference results
and aggregate reference provenance remain open.

## Bounded scope and exact contract delta

Add `ReturnContract::MutWholeParams(Arc<[usize]>)` to the existing canonical
`Arc<RefContract>`. No other signature-sidecar fields or CFG opcodes are needed.
The first mutable-result slice uses explicit named signature regions and only
mutable reference inputs, with nonreference inputs permitted. Existing shared
results keep their all-shared input boundary. Existing B2 noescape functions may
still have mixed reference inputs. Shared inputs to a mutable-result function
are a legal-unsupported boundary initially, not a source of unique authority.

Candidates are the complete sorted original Signature.params indices of mutable
reference inputs with the same pointee type whose declared region outlives the
result. Body universal regions remain rigid. Lifetime dependencies retain every
applicable actual origin, including different-pointee same-region inputs, while
candidate authority remains source-specific. No local, projected, temporary,
static, nested or aggregate reference results are admitted by this slice.

## Required ownership changes

A mutable returned reference is one unique runtime handle with conservative
alternative targets, not one new capability per candidate. CallReturn origin
kind must derive from the actual result BorrowKind rather than being hardcoded
Shared. Every candidate retains only its own ancestor chain. Never union
candidate authority exemptions, upgrade a shared origin, or copy a mutable
reference handle.

At normal Return, prove whole mutable external-root provenance and all universal
lifetime obligations from the read-phase state, then consume the mutable return
handle with Move. Native pointer loading is transport, not semantic copying.
Returning a mutable carrier at an expected mutable-reference site reuses the
existing explicit/synthesized reborrow path before consuming a carrier. An
unannotated `let moved = r` remains a real unique-handle move.

Calls capture target alternatives and lifetime obligations before argument
consumption, keep argument loans live through invocation and preserve B2 mutable
call effects. Invalidate selection proofs below every mutable actual, not only
potentially returned candidates. Initialization stays unchanged. A live returned
unique handle freezes overlapping parent reads, writes, moves and conflicting
borrows. Parent authority resumes after the returned handle's last use when no
other live child retains it. Moving the result transfers origins and invalidates
the old carrier. Shared and mutable child reborrows retain existing freeze rules.

Writes through a union-origin result require all candidate referents initialized
and authorized, preserve initialization bits and kill all may-target proof
subtrees. They must never initialize every candidate as if all were written.
Ambiguous overlap without sufficient correlation fails closed rather than
inventing authority. Caller actual mutable arguments must be mutually
nonoverlapping under the existing conservative overlap relation.

## Source, native and trust seams

Mutable results remain thin 64-bit scalar pointers, not copied referents. Genuine
mutable-reference Call-result temporaries can support Copy pointee reads through
Deref only after unique outgoing-origin seeding, without Copy of the carrier.
Borrowing a call rvalue or using an effectful computed call as an assignment
place remains separately unsupported. The whole-program all-body and exact
signature/analysis gate remains mandatory, including unused invalid callees.
Standalone checked emission continues rejecting reference calls without program
evidence. Mutable source/result kinds must match exact declaration contracts.

## Acceptance before promotion

- Mutable identity writes original scalar, full-width usize and aggregate storage.
- Two disjoint input candidates support either branch and freeze both parents
  as required by their declared result lifetime. Overlapping actuals reject.
- Moving a result invalidates its old carrier. CopyMut and forged shared-source
  mutable contracts reject, including malformed unreachable CFG.
- Return expected reborrows, explicit reborrows, forwarding and recursion retain
  unique authority and actual owner provenance.
- Parent read/write/move conflicts while a child lives, last-use resume, result
  child reborrows and parent-handle death versus owner death are distinct.
- Different-pointee same-region inputs remain lifetime dependencies; independent
  region inputs can end earlier without acquiring result authority.
- Callee mutation of nonreturned inputs and result union writes invalidate all
  applicable proofs without restoring initialization.
- Local results, missing universal outlives and short-lived dependencies reject.
- Empty divergent contracts, SCC precision boundaries and malformed metadata
  remain explicit and fail closed.
- Existing 64 regions, 4096 combined origins, one million memberships, depth/path,
  128 MiB peak-state and 32-million work caps include all new mutable effects.

Projected summaries, mutable-input shared returns, reference-containing
aggregates and general lifetime inference remain following slices. This bounded
implementation is not a full-Rust completion claim.
