# Free-function lifetime elision

Status: frozen after owner feasibility and independent architecture review.
Implementation and acceptance gates are pending. This extends projected reference
returns without changing public IR/CFG/provenance APIs. Full Rust remains open.

## Normalization before body checking

For a free function with a reference result, retain explicit lifetime declarations
in declaration order. Each omitted lifetime on a standalone reference input gets
its own fresh rigid RegionParamId, allocated after explicit names in original
parameter order. Repeated explicit names retain the same ID. Synthetic IDs have
no source spelling and cannot be named by explicit outlives bounds.

An omitted output lifetime resolves only when the set of DISTINCT IDs actually
occurring in reference inputs has exactly one member. Nonreference arguments and
unused explicit lifetime declarations do not contribute. Thus two inputs both
using explicit `'a` permit an elided output, while two omitted input lifetimes
produce two independent IDs and reject an ambiguous omitted output. Zero input
regions also reject a missing output lifetime. Count all reference input regions,
not only parameters whose pointee type can supply a return candidate.

An explicit output lifetime retains its declared ID and rigid outlives rules.
Never infer an anonymous input-to-output bound from body use. Body return proofs
must still establish exact declaration-derived InputPath provenance and every
transitive lifetime dependency. Forwarding and recursion use canonical normalized
declaration contracts, not callee body summaries.

The combined explicit and synthetic region count is capped at64 before growth.
Existing canonical helper validation, closure work and shared byte/work budgets
remain authoritative. NoReference B2 functions keep their existing erased input
regions and do not acquire a 64-reference-parameter regression. Named lifetime
contracts on no-reference-result functions remain a separate existing boundary.

## Unchanged boundaries and ownership

No new AST fields, type variants, CFG operations, native representation or loan
analysis are required. Canonical Arc<RefContract> propagation is unchanged. Source
normalization belongs to frontend.rs, with no production changes expected in
ir.rs/cfg.rs, borrow.rs/ownership.rs or backend.rs.

Returning functions keep the existing same-kind reference input policy. Implicit
field autoderef, mixed-kind result contracts, method lifetime rules, `'_`, named
local annotations, reference aggregates, nested references and unsupported SCC
flows are not enabled by this slice. Unique mutable returns still prove provenance
before Move. Whole/static projected referents, separate lifetime dependencies,
all-mutable-actual proof invalidation and all-body publication gates stay intact.

## Acceptance and exact historical promotion

Use one bounded new stock-classified corpus with shared/mutable identities,
projected/forward/recursive returns, repeated explicit input lifetime uniqueness,
scalar arguments, explicit output with fresh anonymous inputs, missing/ambiguous
output errors, local escape and dependent storage lifetime failures. Check combined
64/65 resource bounds and canonical equivalence with explicit contracts in owner
regressions. Retained graph tests must prove canonical IDs/paths, exact shared CFG
and ANALYZE facts, no OBJECT on CHECK and unused invalid-body publication failure.

Two historical exact sources are expected to become supported: shared
`elided_result` and mutable `elided_mutable_result`. Promote only their existing
classification/assertion after actual new native acceptance, without changing any
source byte, stdout or expected value. All other historical sources and oracle
classifications remain unchanged. No implicit-autoderef promotion is included.

Final gates remain full hosted check/test/strict all-target lint, formatting,
independent actual-code review, tracked eighteen-corpus native acceptance and raw
receipt retention. This is not full Rust completion or a performance claim.
