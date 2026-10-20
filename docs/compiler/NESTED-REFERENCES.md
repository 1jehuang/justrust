# Common shared nested-reference provenance

Status: CONTRACT FROZEN, October16,2026. Implementation requires separately
scoped explicit GO after this contract is committed. The current compiler is
pushed as a1c86fe56 with independently retained1052-case evidence128c12909.
This document is a design and classification contract, not runtime acceptance.

## Goal and dependency

This is a common language, signature, ownership and borrow-analysis foundation,
not a catalog-only exception. It precedes executable canonical shared-reference
Clone. Full Rust compatibility remains the goal. Its first bounded client admits
two shared-reference layers while preserving existing one-layer mutable references
and genuine two-phase borrowing unchanged.

No compiler code, corpus source, build, probe or test is authorized by this draft.
Exact API and independent stock classification must be frozen before source GO.

## External first-client contract

- Admit `&T` and `&&T`, including explicitly named independent lifetimes, where
  the innermost referent obeys existing reference-free type formation rules.
- Rigid generic referents remain opaque, implicitly Sized ordinary parameters.
  Source generic type actuals remain recursively reference-free.
- Preserve all existing one-layer `&mut T`, borrowing, method dispatch, return,
  move, storage, two-phase activation and diagnostic semantics.
- Initially reject chains containing a mutable reference, reference-containing
  aggregate fields/elements, more than two reference layers and source reference
  type actuals as explicit legal unsupported boundaries.
- No implicit permission to inspect opaque layouts, extend temporary lifetimes,
  fabricate reference values or suppress an ordinary ownership error.
- Inner references may escape a temporary outer reference-handle scope when their
  original referent remains live. They may not escape the original referent.

## One position vocabulary

The agreed shared IR vocabulary is:

```rust
struct ReferencePosition { depth: u8 }
struct InputReference { param: usize, position: ReferencePosition }
```

Depth zero identifies the outer reference value. Depth one identifies the inner
reference value of the initially admitted two-layer shared chain. Private analysis
uses `CarrierSlot { local, position }`, distinct from canonical memory places.

Input paths and envelopes select an `InputReference`, not a bare parameter index.
Canonical external formal referent roots are position-qualified. Every checked
reference position, including call results and dead metadata, must be validated.
No public boolean or caller-supplied evidence may bless a position.

The single coordinated contract representation is:

```rust
struct InputRegion { input: InputReference, region: RegionParamId }
struct ResultReference {
    position: ReferencePosition,
    region: RegionParamId,
    returns: ReturnContract,
}
struct RefContract {
    region_count: u16,
    input_regions: Arc<[InputRegion]>,
    results: Arc<[ResultReference]>,
    outlives: Arc<[(RegionParamId, RegionParamId)]>,
}
struct InputPath { input: InputReference, projection: Arc<[Projection]> }
struct InputEnvelope { input: InputReference }
```

The existing concrete projection element representation remains unchanged. Input
rows are sorted by parameter then depth, result rows by depth, with exact dense
coverage of typed positions. Reject duplicate, missing and extra positions.
Non-reference results have no result rows. `ReturnContract::NoReference` is only
a raw/default placeholder and never a published ResultReference payload.

`normalize_ref_contract(raw: &RefContract, budget: &PlanBudget)` retains its
existing `Result<Arc<RefContract>, String>` return. The one rigid declaration
helper becomes `derive_return_contracts(params: &[Ty], result: &Ty,
regions: &RefContract, budget: &PlanBudget) -> Result<Arc<[ResultReference]>,
String>`. It fills each result mapping while preserving normalized result positions
and regions. No specialization rederives those mappings.

The existing high-arity unnamed free-function non-reference-result erasure path
is preserved as a narrowly authoritative exception: empty input/result rows are
accepted ONLY when the exact source declaration/schema qualifies for that legacy
mode. There is no caller-supplied skip flag. Named-region, borrowed-receiver,
nested-reference and reference-result signatures do not qualify. Runtime carrier
positions remain type-derived even when this legacy contract has no region rows.
The 64-region cap is not increased and the existing high-arity admission is not
silently narrowed. Exact qualification is checked by the canonical world subject
service, never by trusting an arbitrary empty contract.

The legacy representation is exactly zero regions, empty input/result rows and
empty outlives edges. Eligibility is derived from original proof-free canonical
source schema metadata and verified during world bootstrap, not merely asserted
by an arbitrary raw HeaderSet. The authoritative source declaration metadata uses
`RegionContractMode::{Positioned, LegacyUnnamedFreeNoReturn}` with field
`region_contract_mode`. This is not a RefContract mode, checker skip flag or
caller-provided proof. It is computed before normalization from the original
source signature, including declared-but-unused lifetimes and every explicit
reference annotation. Exact canonical raw declaration schemas must also be obtained
from the authoritative adapter, rather than comparing a HeaderSet against its own
substitution. The exact additional object-safe query is
`SchemaCx::declaration_schema(&self, id: DeclId) -> Result<GenericSchema,
FactsError>`. Existing `declaration_info` supplies `region_contract_mode`.
Both are proof-free raw registry authority, never published-world callbacks.
The query covers executable callables and required trait item signature schemas.
Specialization transports the declaration decision unchanged.

## Lifetimes and declaration authority

Every elided input reference occurrence, including inner occurrences, introduces
its own lifetime before ordinary elision rules apply. Free output elision needs
one input reference lifetime occurrence under the existing independently verified
E1 rule, not a deduplicated set of RegionParamIds. Borrowed receiver output elision
uses its outer receiver lifetime. Never infer an inner lifetime by copying its
enclosing one.

Every input and result reference position has its own region metadata. Formation
of `&'outer &'inner T` contributes the implied `inner: outer` well-formedness
relation before complete region closure, mediator pruning and canonical IDs.
Canonical first-use order includes parameter and reference position, then result
positions. Names and declaration order are not semantic identity.

Rigid declaration normalization and return-origin derivation happen once. Their
position-qualified origin choices and region relationships are transported through
substitution unchanged. Concrete type coincidence never adds positions, paths or
parent authority. The existing canonical Body/Call signature service remains the
sole production signature authority, with private proof-backed trait refinement
and all-body closing preserved.

## Actual reference values, not footprint substitution

Loading the inner value through `&&T` first checks the outer read and its access
authority. It then transports the stored inner value's origins, candidate-specific
parents and lifetime dependencies. It does not derive inner origins from the outer
reference's overlap footprint, a currently rebound local, or a fictitious field.

The temporary outer loan and reference-handle storage are not permanently added
to the returned inner value's pointee dependencies. Conversely, their access must
still be valid at the load. A value snapshot cannot later acquire origins from a
new assignment to the source handle.

Root Copy and Move transport all reference-value slots. Borrowing a stored
reference snapshots its value into the inner position of the resulting carrier.
Storage death kills all corresponding slots. Rebinding, control-flow joins,
last-use liveness, calls, returns and genuine call-result temporaries transport
position-specific values through the same existing solver.

A nested identity maps result depth zero to input parameter zero depth zero,
and result depth one to input parameter zero depth one. An inner-return operation
maps result depth zero to input parameter zero depth one. `&r` creates the normal
outer loan at result depth zero and snapshots the stored `r` reference value into
result depth one. Copying `*rr` reads the outer referent and transports source
position one to destination position zero. These are typed value operations, not
new semantic Copy permissions.

These examples describe actual value-slot provenance, not a promise that canonical
return candidate lists are singletons. For EACH result position independently,
preserve the existing rigid declaration rule: if an eligible input referent or that
result position's referent contains opaque Param/Self, use all eligible
InputReferences in its envelope. Otherwise retain complete exact paths. Transport
those candidate rows unchanged through substitution. Each actual returned slot
must satisfy its corresponding canonical row, with no post-substitution derivation.

Formal roots use the authoritative referent type at each selected position.
Parameter origins enumerate all authoritative typed formal positions, NOT the
input-region rows: legacy erased signatures still require their full carrier origins
and normal conflict, move and initialization checks. Region lookup applies only to
positioned lifetime contracts and return obligations. Call-return origins expand
per typed result position. Every dereference
step selects the correct carrier value before ordinary reference-free projections.
Live-use accounting separately includes outer read access and the inner value use.
No result position may be omitted merely because native representation is a single
pointer. Every call and return contract row is checked, including unreachable IR.

The current Exact/Subtree distinction remains separate from typed returned views.
Do not append a returned view's field coordinates to an unrelated input footprint.
Do not union candidate authority exemptions. Ordinary explicit Move still consumes
when physical shared-reference copying would also be permitted.

## Same analysis, facts and seals

Extend the existing Paths, origins, liveness, lifetime dependencies and borrow Plan.
There is no second reference checker, catalog special solver or native shortcut.
ValidatedFacts continues to bind exact world, registry, declaration/instance,
environment, predicates, phase and the same PlanBudget. Required trait signatures
remain schema-only. All executable bodies and all concrete instances retain their
current validation and concrete-proof publication barrier before native emission.

Reference-position metadata is compile-time analysis. Native thin-pointer ABI
remains unchanged, but an inner load must be a genuine typed pointer load, not an
invented Clone operation or bitwise aggregate substitute.

## Resource and cache contract

Keep current caps, including combined 4096 total origins and one million retained
memberships. Formal positions and call-result positions count in those totals.
No silent per-position cap multiplication or cap increase is permitted.

Before allocation, use checked arithmetic to preflight expanded carrier slots,
formal/result positions, point-by-slot origin bitsets, liveness, activation,
dependency sets, joins, worklists, snapshots and relocation peak storage. Charge
all copies, scans, rejected metadata and cache hits to the existing shared ledger.
Inner loads copy bounded existing inner-slot origin sets, not fresh recursive origin
families on every use. Memo keys include reference position/depth, type-use
context, exact declaration/subject authority and current world/environment as
required. Type equality alone is not a provenance cache key.

## Acceptance and scope discipline

Before source GO, freeze one complete API and exactly32 independently stock-
classified cases. Preserve sources, stdout, categories and initial oracle artifacts.
Required positives include direct inner returns, nested identity, generic reference-
free referents, independent lifetimes, inner escape from temporary outer handles,
value snapshots across rebinding, genuine calls evaluated once and joins/last use.
Required negatives include original referent death, moved/uninitialized outer
carriers, conflicting outer access, wrong positional origins and region escape.

After owner production and existing-fixture checkpoints, permit only explicitly
bounded grouped regressions. Run hosted checks, all preserved tests, strict lint,
format and diff checks. Validate actual native workers one and eight using the
frozen stock corpus and cumulative historical campaign. Retain raw receipts and
path-redacted profiles. Design clearance is not patch or runtime acceptance.

## Exact independently classified oracle inventory

Stock classification is complete for these exact32 cases. No source correction,
category change or case growth is authorized after this freeze. Observed counts
are16positive,12invalid,4legal-excluded. They are independent stock classifications,
not acceptance by this compiler.

| ID | Category | Case |
|---|---|---|
| P01 | positive | nested_shared_identity_preserves_both_carrier_layers |
| P02 | positive | load_inner_transports_original_referent_origins |
| P03 | positive | loaded_inner_escapes_temporary_outer_handle_scope |
| P04 | positive | loaded_inner_escapes_local_inner_binding_scope |
| P05 | positive | generic_reference_free_T_inner_load_and_forward |
| P06 | positive | plain_shared_carrier_copy_with_noncopy_reference_free_T |
| P07 | positive | distinct_named_lifetimes_imply_inner_outlives_outer |
| P08 | positive | explicit_inner_result_lifetime_not_outer_lifetime |
| P09 | positive | call_returns_inner_with_original_argument_origins |
| P10 | positive | genuine_nested_call_once_forwarding_preserves_layer_positions |
| P11 | positive | joined_inner_load_unions_original_referent_origins |
| P12 | positive | rebind_nested_carrier_then_load_uses_current_origins |
| P13 | positive | loaded_inner_survives_handle_rebind_to_other_referent |
| P14 | positive | referent_write_after_loaded_inner_last_use |
| P15 | positive | handle_rebind_after_outer_last_use_while_loaded_inner_lives |
| P16 | positive | T_U_concrete_coincidence_preserves_inner_candidate_authority |
| N17 | invalid | loaded_inner_escapes_original_referent_storage |
| N18 | invalid | returned_outer_escapes_local_inner_handle_storage |
| N19 | invalid | live_loaded_inner_blocks_original_referent_write |
| N20 | invalid | live_loaded_inner_blocks_original_referent_move |
| N21 | invalid | live_loaded_inner_blocks_original_referent_exclusive_borrow |
| N22 | invalid | live_outer_blocks_inner_handle_rebind |
| N23 | invalid | uninitialized_outer_carrier_inner_load |
| N24 | invalid | joined_inner_retains_expired_branch_origin |
| N25 | invalid | loaded_inner_rebind_does_not_release_old_live_origin |
| N26 | invalid | unused_generic_body_returns_inner_under_unrelated_result_region |
| N27 | invalid | implied_inner_outlives_outer_does_not_prove_reverse |
| N28 | invalid | wrong_inner_position_candidate_despite_T_U_coincidence |
| X29 | legal-excluded | nested_mutable_reference_chain |
| X30 | legal-excluded | aggregate_reference_fields |
| X31 | legal-excluded | ordinary_reference_generic_actual |
| X32 | legal-excluded | shared_reference_depth_greater_than_two |

Named output regions avoid ambiguous free-function nested-reference elision.
P04 ends reference-handle binding storage, not original referent storage. N23 is
uninitialized rather than a falsely assumed consuming shared-reference assignment.
N27 requires an unjustified reverse relationship in the rigid body, not malformed
input formation. No case depends on canonical Copy or Clone being implemented.

## Frozen stock provenance

The final observed corpus is
`/home/jeremy/.jcode/scratch/nested-references-oracle/corrected/corpus.observed.json`,
SHA256 `0554a1035124644fb649691423b3706888c4aa6395437f92c493c298a6b49432`.
Its source-only corrected artifact SHA256 is
`75fc425a83eea8e4f835a9a17e98636c78a80ba54afd7512958f3fd60f0a38a6`.
All32 sources, stdout, integer expectations, categories and hashes are frozen.

Independent stock BUILD and genuine metadata-only CHECK both classified all32.
The20 valid cases retained executable BUILD artifacts and actual CHECK metadata,
then ran with exact literal stdout, empty stderr and zero exit. All12 invalids
failed BOTH phases with exit1 and no artifacts. N26/N27 retain exact uncoded
structured error message `lifetime may not live long enough`, not fabricated codes.
Other invalid codes in order are N17 E0597, N18 E0515, N19 E0506, N20 E0505,
N21 E0502, N22 E0506, N23 E0381, N24 E0597, N25 E0506 and N28 E0308.
Actual multiplicities and full raw JSON diagnostics are retained.

Root separately rebuilt, metadata-checked and executed the same frozen source
bytes, all32 passing independent classification. Root receipts SHA256 is
`550b610d926ba20dda7852b7a54946b3a7bfb4286f8e32512e5412acef17f9a0`.
This is stock-oracle evidence only, not native acceptance or a speed claim.

Original generated artifact SHA256
`766330d6c9afab85ee79aabf5d3d161b3f1b457876521979e640e39be660e60f`
and all original source files remain unchanged. Before stock observations, root
explicitly authorized ONLY removing generated allow-attribute scaffolding and
replacing unsupported formatted-print wrappers with same-point scalar observation
snapshots and fixed literal output. Borrow/load/call/branch/rebind/write/move
operations, their order and last-use points were preserved. A separate per-case
before/after mapping records the correction, SHA256
`25328bf72e592760c52badd4149325e14fe509d30fadb68be8839ff845794ed8`.
No stock bucket mismatch required a semantic source repair. Final metadata only
adds observations and changes classification_observed from false to true.

## Subsequent canonical Copy and Clone client

Trusted catalog identity, real supertraits, Copy implementation eligibility and a
common checked builtin-body provider remain the next client. No source trait
spelling grants Copy authority. Shared-reference Clone execution is not delivered
until this common nested-carrier foundation is validated. Mutable indirect carrier
store and the provided canonical `clone_from` body remain a separate explicit
execution boundary unless genuinely implemented and independently checked.
