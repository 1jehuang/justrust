# B2: noescape reference parameters and expected-site reborrows

Status: implemented with 748 hosted tests, strict all-target lint, formatting
and independent source/resource review passing. The frozen 30-case stock corpus
contains 18 successful programs, eight invalid programs and four explicit
unsupported boundaries. Tracked fourteen-corpus native verification passed all
602 cases, with raw receipts retained under
`experiments/results/20261016-m8a-reference-calls-*`.
B1 is recorded in `BORROWING.md`. Full Rust and escaping lifetimes remain open.

## Scope

Admit standalone shared and mutable reference parameters on direct free functions.
Their referents must recursively contain no references. Keep reference results,
reference-containing aggregates, nested references, globals, opaque calls, FFI,
explicit lifetime syntax and escaping references unsupported. Validate these
boundaries even for unused declarations and malformed private CFG. Preserve all
B1 projection, dynamic-index, rvalue-borrow and SCC precision restrictions.
Variant-pattern matching through a dereference remains explicitly unsupported:
selection proofs are not keyed by reference-carrier slots. Structural CFG
validation rejects dereferencing SwitchVariant places before static refinement.

No new public CFG instruction is required. `LoanId` continues to index explicit
`Function.loan_sites` only. Formal parameter origins are not CFG loan IDs.

## Canonical formal referents

The borrowing analysis distinguishes owned storage from formal external storage:
`CanonicalRoot::{Owned(LocalId), ParameterReferent(LocalId)}` and a canonical
place consisting of that root plus static projections. A parameter carrier slot
and its referent are different roots, even when identified by the same LocalId.
Moving or overwriting the parameter handle must not invalidate or retarget aliases.
No fake stack locals or runtime metadata are introduced for abstract referents.

Internal origin IDs span explicit loan sites followed by reference parameters in
stable parameter order. Seed each reference parameter carrier with its own entry
origin. Formal origins have kind, a canonical external target, no parent and no
in-function birth. Every dense origin/parent/target structure uses the combined
origin count, including functions with zero explicit borrow sites.

Build separate bounded move-path roots from formal referent types. External
referents are live and fully initialized for the invocation, with unknown enum
variant selection except existing singleton rules. Carrier StorageDead does not
end external storage. Shared formal referents are read-only, mutable formal
referents writable by their own authority regardless of carrier binding mutability.

Owned frame roots are disjoint from external roots. Distinct mutable formal
referents are disjoint from other formal referents by caller-checked safe argument
contracts. Shared formal referents may alias. Within a root, preserve existing
field/prefix/array overlap rules. Resolve reborrows through formal origins with
per-candidate ancestor authority, never unioned exemptions.

## Expected-site reborrowing

At known expected reference sites, a direct local mutable-reference carrier is
reborrowed without moving it: expected `&mut T` creates a mutable borrow of
`r.Deref`, and expected `&T` creates a shared borrow. Shared to shared is ordinary
Copy. Explicit borrow expressions remain their own sites. Apply to call arguments,
annotated lets and assignment into a known reference local. An unannotated
`let q = r` remains a genuine mutable-handle move.

Explicit `&mut place` supplied to expected shared reference remains an explicit
unsupported coercion in this slice. Only a direct mutable carrier synthesizes a
shared reborrow. Explicit borrow kinds are not silently changed.

Do not materialize or move a carrier and then pretend a reborrow preserved it.
Complex block/if/match reference coercions must propagate context to their actual
tails and arms, or fail explicitly as unsupported. Preserve source evaluation order.

## Calls and effects

Evaluate arguments left to right into stable owning temporaries. Reference
argument temporaries remain live through the call, including when formal
parameters are unused. Earlier argument loans must therefore conflict with later
argument effects when appropriate. Same shared arguments may alias; overlapping
mutable/shared and mutable/mutable arguments reject; disjoint ordinary fields can
be passed mutably. Repeated calls using a mutable carrier create separate child
reborrows whose lifetimes end after each call.

Add origin transfer for Call terminators: capture actual canonical targets before
consuming Move reference temporaries, then clear moved carriers on the outgoing
edge. A no-reference result never seeds returned reference origins.

A mutable-reference call may mutate or replace its referent. On successful return,
kill variant-selection proofs below every candidate mutable actual target before
continuation or destination write. Preserve initialization bits, and never
initialize every possible alias target. Shared calls do not invalidate proofs.
Capture targets before consuming argument temporaries, and do not reread moved
carriers afterward. Apply this same rule for nested and recursive calls through
formal external roots.

Noescape prevents reference escape, not referent mutation. Full initialization
survives a returning safe call because moving non-Copy values out of dereferences,
Drop, unsafe and reference-containing values remain forbidden. Panic has no normal
continuation in the current runtime.

## Native and graph contracts

References remain thin 64-bit scalar parameters, not copied aggregate referents.
Formal external roots are analysis-only. Calls with aggregate referents must
mutate original caller storage. Existing nonreference aggregate-result ABI stays
unchanged. Raw CFG validation must enforce noescape signature restrictions and
actual argument types, not trust unsupported signatures.

Standalone checked-function emission rejects calls with reference parameters or
results without whole-program declaration evidence. It may emit an analyzed
reference-parameter body with no such calls. Whole-program `emit(&Program)` uses
one canonical TypeEnv, lowers and analyzes every body exactly once before raw
emission. Trusted raw CFG emission additionally requires whole-program noescape
and callee-signature evidence for reference calls. Local CFG validation checks
structure and signature shapes, not callee existence or correctness.

CHECK analyzes all function bodies. BUILD must pass all bodies before publishing
an executable, including unused bad callees. Individual body lowering/emission
still depends on declarations and its exact analysis fact, not callee body facts,
so recursion introduces no graph cycle or duplicate analysis.

## Resource and correctness gates

Initial combined limit: 4096 explicit-site plus formal-parameter origins, not
4096 of each. Retain one million total origin/target/parent memberships, 128 MiB
peak state including scratch, 32-million shared work budget, 128-depth and
131072 move-path limits. Bound expansion of external referents before allocation.
No-site functions with many reference parameters must still account for seeded
origin state, parent bitmaps, paths, joins and call-target scratch. Exhaustion
fails closed and never publishes a passing incomplete analysis.

Acceptance must cover zero-site formal reads/writes, mutable pointee through an
immutable parameter binding, shared aliases, conflicting arguments, disjoint
fields, explicit and implicit reborrows, genuine untyped handle moves, parameter
rebinding and surviving children, forward/nested/recursive calls, argument effect
order, referent proof invalidation, unchanged initialization, all-body/noescape
failures, malformed provenance/signatures, combined caps and full-width ABI.
Run stock accepted/invalid/legal-unsupported classifications, native workers 1/8,
full regression/lint gates and tracked raw differential receipts before promotion.

## File ownership after explicit GO

- Frontend owner: signatures, expected reference sites and noescape declarations.
- IR/CFG owner: signature validation and reference argument lifetime guarantees.
- Borrowing owner: borrow.rs and ownership.rs together, canonical roots, formal
  origins, external paths and call effects.
- Backend owner: thin ABI audit and native call/reference regressions.
- Acceptance owner: new bounded stock-classified reference-call corpus.
- Coordinator: graph integration checks, documentation and retained receipts.

B3 reference returns, signature regions/outlives, reference-containing aggregates,
Drop and general Rust lifetime inference remain explicitly later work.
