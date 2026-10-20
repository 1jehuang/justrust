# Executable generic carrier values and storage effects

Status: **Contract freeze candidate, independently stock-classified.** This document is not source,
probe, build, test, stock-oracle or implementation authorization. The previous
canonical Copy/Clone implementation and independent evidence are committed.
The next stage remains unimplemented until a separately scoped authorization
follows contract review, independent stock classification and contract commit.

This is one common bounded foundation for executable reference-valued generic
arguments, Source bodies and Recipe bodies. It is not a Clone exception, an
alternative borrow checker, unrestricted generic reference support or a claim
that full Rust is complete. The externally observable Rust contract remains
the requirement. Existing source admission must not be weakened by invented
loan authority, type-equality shortcuts or an unverified body summary.

The records below were consolidated by the IR, ownership, world and frontend
owners. Their design feasibility acknowledgments are not runtime acceptance.
Later sections define unchanged proof/resource gates and a provisional oracle.

## 1. Shared structural records

```rust
enum CarrierChannel {
    Fixed(ReferencePosition),
    ParamTail { parameter: ParamId, prefix: u8 },
}
enum CarrierStep {
    Field(usize),
    ConstantIndex(usize),
    VariantField { variant: VariantId, field: usize },
    Referent,
}
struct CarrierPath(Arc<[CarrierStep]>);
struct CarrierLocation { path: CarrierPath, channel: CarrierChannel }
struct InputCarrier { param: usize, location: CarrierLocation }
struct CarrierReturn { location: CarrierLocation, sources: Arc<[InputCarrier]> }
struct CarrierWriteEnvelope { destination: InputCarrier, sources: Arc<[InputCarrier]> }
struct CarrierContract {
    inputs: Arc<[InputCarrier]>,
    results: Arc<[CarrierReturn]>,
    writes: Arc<[CarrierWriteEnvelope]>,
}
```

ParamTail is an elementwise family of ALL latent components of the exact original owner-qualified Param, not a fabricated depth-zero reference. Value occurrences are distinct: x:T and y:T have different entry origins. A family has no concrete memory target, BorrowKind, LoanId, layout or Copy permission. Hidden component identity includes (ParamId, CarrierLocation), not only relative reference depth.

CarrierPath is a typed VALUE lens, not a CanonicalPlace memory projection. The authoritative type walk establishes one normal form: leading reference chains are absorbed into channel position/prefix, with Referent steps for reaching aggregate components beyond those chains. Duplicate encodings are invalid. Array/tuple/nominal/variant construction, projection and partial move transport distinct component paths.

The CarrierContract SUPPLEMENTS the existing fixed RefContract. It never erases InputPath memory projections, Exact/Subtree target precision, per-candidate authority, parents or lifetime dependencies. Value origin transport is not memory target authority.

## 2. Canonical world view and nominal transport

```rust
// Existing ValidatedFacts, one readonly service:
fn carrier_contract(&self, subject: SignatureSubject<'_>)
    -> Result<Arc<CarrierContractView>, FactsError>;
// CarrierContractView readonly accessors:
// contract() -> &CarrierContract
// origin_schema() -> &Arc<SignatureOriginSchema>
// ref_contract() -> &Arc<RefContract>
// region_schema() -> &CarrierRegionSchema

// Private structural transport record:
struct NominalCarrierMapping {
    original: NominalAppId,
    instantiated: NominalAppId,
}
// Private, inside the SAME structural view/topology certificate:
struct CarrierComponentMapping {
    original_parameter: ParamId,
    original: CarrierLocation,
    substituted: CarrierLocation,
}
```

The view has private exact binding to world identity, registry SnapshotId, declaration DeclId, original canonical schema, ordered Arc<[Ty]> actuals, authoritative environment index, FactsPhase and SAME PlanBudget authority. Only the world constructs it. Existing world-owned empty-context sentinel is handled explicitly, never indexed as an ordinary environment. No caller bool or raw signature mints a view.

World builds origin schemas for ALL original callable declarations BEFORE bodies and independently compares declaration_schema. This is header authority, not executable family proof. Calls mapping callee U to caller T compose the exact owner-qualified family and prefix, never retain foreign U or identify distinct T/U from equal concrete types. World never calls back into VerifiedTemplateCfg.

NominalCarrierMapping lives inside existing structural topology/carrier-view proof. SAME canonical interner validates exact source declaration, ordered substituted arguments, field ordering and variant declaration/index correspondence before retaining the pair. No copied layout, accidental AppId equality or public callback grants authority. Internal Wrap<T>/enum/array construction after an admitted root reference actual uses this SAME certified substitution path. It does not admit external Nominal<Refs> source syntax or explicit reference fields without their declaration lifetime model. The private internal interner entry must be subject-bound, not a general validate_argument waiver. Source and Recipe use the same path.

## 2a. Body-local substitution authority

```rust
// Private proof sidecar, never a world-owned callback or interner permission:
struct BodySubstitutionProof {
    template: VerifiedTemplateCfg,
    target: CallTarget,
    nominals: Arc<[NominalCarrierMapping]>,
}
```

Exact closed world/snapshot/environment/phase/ledger binding is derived from the retained template and canonical target facts and checked by the private constructor. instantiate_checked uses the SAME exhaustive typed walker to collect mappings for ALL body-local, arena, place, pattern, call and dead metadata, not merely signature types. It checks canonical source declaration, substituted arguments and field/variant correspondence before creating the proof. The returned checked body retains Arc<BodySubstitutionProof> in its proof sidecar. Lowering validates exact template/target/facts and ALL corresponding type occurrences, and retains the same proof through CFG structural and analysis wrappers. The proof retains only the ORIGINAL verified template/source, never its produced output, avoiding a source/proof backedge. No world-to-body dependency, second visitor, global interner-as-permission or unsealed candidate-cache authority is introduced. Source syntax admission precedes interner lookup. This applies equally to Source and Recipe bodies.

## 3. Original named regions and per-call evidence

ROOT FINAL spelling is source_region_env with dense normalized mapping. All declaration_region_env/fixed_bindings alternatives are withdrawn.

```rust
struct SourceRegionId { declaration: DeclId, index: u16 }
enum ActualRegion { Named(SourceRegionId), Elided }
struct SourceRegionEnv {
    declaration: DeclId,
    region_count: u16,
    outlives: Arc<[(SourceRegionId, SourceRegionId)]>,
    normalized: Arc<[Option<RegionParamId>]>,
}
// On proof-free authoritative SchemaCx:
fn source_region_env(&self, declaration: DeclId)
    -> Result<Arc<SourceRegionEnv>, FactsError>;

enum CallExpectation { NoFirmReference, FirmReference { kind: BorrowKind } }
struct CallArgumentTrace { argument: usize, value: ExprId, expectation: CallExpectation }
struct TypeActualRegion { argument: usize, location: CarrierLocation, source: ActualRegion }
struct CallEvidence {
    call: ExprId,
    arguments: Arc<[CallArgumentTrace]>,
    type_actual_regions: Arc<[TypeActualRegion]>,
}
// CheckedFunction.call_evidence: Arc<[CallEvidence]>
```

region_count is the ORIGINAL named declaration count, capped before growth. normalized is dense by original index. None means a still-rigid original named variable, not Elided. Names stay frontend-private. World validates owner/index, original named outlives, normalized schema agreement, snapshot and ledger before publication. Catalog/recipe rows have zero source-named regions and their ordinary fixed contract.

A named variable owned by the currently analyzed caller is rigid, including normalized None. A CALLEE declaration lifetime is rigid in its own body, but freshly instantiated at EACH caller occurrence and constrained by actual origins and caller Named evidence. This role distinction also applies to recursive calls where caller and callee have the SAME declaration and SourceRegionId: the caller binder remains rigid, while the callee-use instance is fresh. Reusing SourceRegionId across calls does not reuse permission. Each Elided occurrence is fresh. CallTarget/native keys remain lifetime-erased.

Argument traces are dense and match the exact Call argument ExprIds. Original-stage validation derives firm reference expectation from the ORIGINAL canonical callee schema and admitted SOURCE-STAGE explicit/rigid actuals before argument checking. It never derives from final monomorphic Ty or later argument evidence. Existing Borrow/Local/Deref nodes and BorrowTiming record the established adjustment, and remain immutable through instantiation. Newly inferred structural reference actuals are initially Unsupported; existing rigid/reference-free inference is preserved.

```rust
// Private, retained only in proof/Plan sidecars, not inside source it retains:
struct ValidatedCallEvidence {
    source: Arc<CheckedFunction>,
    call: ExprId,
    facts: Arc<ValidatedFacts>,
    trace: CallEvidence,
}
```

Its private constructor validates original source/call, source-stage substitution and trace against authoritative declarations. Original target and source type terms are retained in source, not copied as Params into concrete executable metadata. VerifiedTemplateCfg binds this capsule to exact source. Instantiation transports the verified capsule and stable ExprIds. Nongeneric publication uses the SAME validator. Closed zero-Param boundary stays unchanged.

An original generic capsule has Universal facts, while an instantiated call binding has Closed facts. The phase/environment transition is authorized ONLY through the exact VerifiedTemplateCfg, retained BodySubstitutionProof, canonical target, ordered substitution and source-call-to-CFG correspondence under the same world/registry/ledger. Do not require literal phase/environment equality between source provenance and the closed analysis, and do not ignore a mismatch. Nongeneric original Closed evidence uses the direct exact-facts path. Source Params remain solely in original proof provenance, not executable closed metadata.

## 4. Region-family schema and fresh occurrence binding

```rust
enum CarrierRegionTerm {
    Fixed(RegionParamId),
    Named(SourceRegionId),
    Family { parameter: ParamId },
}
struct CarrierRegionConstraint { longer: CarrierRegionTerm, shorter: CarrierRegionTerm }
struct CarrierRegionSchema { constraints: Arc<[CarrierRegionConstraint]> }
```

Family-to-Fixed/Named applies to ALL typed actual component regions, including implied formation obligations. Family relations require witnessed POINTWISE original-component correspondence, never Cartesian products or concrete T/U equality. The private canonical substitution walk retains Arc<[CarrierComponentMapping]> in the existing structural certificate, bound to original declaration/env/ordered arguments and NominalCarrierMapping. original carries the original parameter-qualified channel/path; substituted is the exact canonical expanded Fixed location or caller-qualified ParamTail location. Its private constructor validates the original_parameter matches the original family, then derives the destination solely by that substitution walk. A Family-to-Family edge is usable only through this retained witness, with full (ParamId, CarrierLocation) component identity; no foreign callee U remains as caller permission. This is not a new solver or public callback. Mutable invariance uses two directed constraints/equality for each matched component, not one-way outlives or origin union. Structural world caches contain no occurrence permission.

## 4a. Existing erased legacy subjects and region allocation

LegacyUnnamedFreeNoReturn remains the EXISTING authoritative declaration-only mode: canonical zero region_count, input/result region rows and edges, unnamed free single-layer reference inputs, nonreference result, with exact source qualification verified by the world. Its typed formal carrier positions/origins are ALWAYS built from types, preserving the existing 4096-origin gate and full initialization, move, alias/conflict, liveness and access checks. Zero region rows never mean zero carriers or skipped analysis.

CallCarrierBinding allocates AnalysisRegionIds only for ACTUAL declared or induced lifetime, return or carrier-store constraints. It does not allocate a normalized region node merely because a legacy fixed input has a typed reference slot. In an exactly qualified legacy call with no such obligations, regions is empty while entry origins and target checks are complete. This does not coalesce the 4096 input lifetimes into one synthetic alias, invent missing contract rows, or raise the 64-region cap. Borrow origin/liveness identity remains independent of normalized contract region identity.

New latent reference actuals, result topology or carrier-store effects require their genuine family/occurrence formation, variance and escape constraints under unchanged caps. They cannot inherit eligibility from a transported erased zero contract or hide behind the old reference-free fallback. The authoritative body/call view derives whether those NEW obligations exist and selects ordinary constrained binding or Unsupported/resource failure as appropriate. No caller flag, empty input rows or cache presence selects the legacy fast path. Original source qualification plus exact canonical subject agreement remains mandatory.

## 5. SAME-Plan typed origins, cells and private call binding

```rust
struct ReferenceOriginId(usize);
struct FamilyOriginId(usize);
struct ReferenceOriginSet(Set);
struct FamilyOriginSet(Set);
struct CarrierOrigins {
    references: ReferenceOriginSet,
    families: FamilyOriginSet,
}
struct FamilyOrigin { input: InputCarrier }

enum CellChannel { ReferenceValue, ParamFamily(ParamId) }
struct CellKey { storage: CanonicalPlace, channel: CellChannel }
// Private CanonicalRoot variant replaces old position-only qualifier:
// ParameterReferent(LocalId, CarrierLocation)

struct AnalysisRegionId(usize);
struct CarrierEntrySnapshot { input: InputCarrier, origins: CarrierOrigins }
struct CarrierDestinationBinding {
    destination: InputCarrier,
    targets: Vec<Target>,
    old: Vec<(CellKey, CarrierOrigins)>,
}
struct CallRegionBinding {
    schema: CarrierRegionTerm,
    component: Option<CarrierLocation>,
    region: AnalysisRegionId,
}
struct CallCarrierBinding {
    source: Arc<CheckedFunction>,
    function: Arc<cfg::Function>,
    call: ExprId,
    block: BlockId,
    facts: Arc<ValidatedFacts>,
    contract: Arc<CarrierContractView>,
    evidence: Arc<ValidatedCallEvidence>,
    entry: Vec<CarrierEntrySnapshot>,
    destinations: Vec<CarrierDestinationBinding>,
    regions: Vec<CallRegionBinding>,
}
```

Set is the existing bounded membership implementation. Typed wrappers separate concrete origin and family universes. Only ReferenceOriginId can address concrete kind/target/loan tables. FamilyOrigin names an original ParamTail input occurrence and universally quantified component relation, never a fabricated concrete loan. SAME transfer/worklist/join algorithms transport both typed domains. Cell layout validates the permitted domain after certified structural retargeting.

CellKey has NO physical reference-depth index. A typed lens loads CURRENT reference-value cell contents and then follows immutable origin targets at every Ref edge. A child store changes that cell's contents; a later parent reload observes it. Alias views of the same canonical storage+channel see one cell. Immutable external entry anchors never rebind. Old ParameterReferent positions import as empty path + Fixed(position). Full CarrierLocation distinguishes field-contained formal anchors without pretending it is a memory projection.

Borrow creates a handle/storage lens, not a frozen copy of mutable pointee contents. Value Copy/Move transports owned value cells with existing consumption rules. Dereference reads outer handles transiently, then loads current inner origins without adding the outer handle as permanent returned authority. Subtree/unknown aliases require a finite COMPLETE set of ORIGINAL typed candidate CellKeys; otherwise Unsupported. No speculative overlays, guessed cells from type equality or post-substitution candidate discovery are admitted. Typed lens resolution retains the per-candidate intermediate handle trace. Issuing a reborrow through a mutable chain retains ALL required intermediate mutable parent/unique-access and lifetime dependencies, not merely the last static dereference slot. Independent shared-reference value loads still do not permanently inherit transient outer-handle dependencies. Read traces and newly issued reborrow dependencies remain separate and candidate-correlated.

The private CallCarrierBinding constructor checks exact original source/ExprId to actual CFG Call block mapping, arguments, validated evidence, canonical contract, caller world/env/phase/registry and SAME ledger. Each call allocates fresh AnalysisRegionIds and binds actual input occurrences and typed invariant constraints. Family component Some must denote an expanded Fixed typed location, not an unresolved ParamTail passed as a concrete component. Fixed/Named terms use None. Symbolic family relations remain separately quantified before expansion.

Bindings are retained only by Plan/proof sidecars, never by Function/source they strongly retain. Dead Call metadata is validated fully, but cannot manufacture reachable entry origins or loan permission. Reachable runtime binding uses actual analysis state. Snapshot every input, destination OLD contents and target lens before consuming source operands.

## 6. Entry semantics, effects, recursion and proofs

CarrierReturn.sources and CarrierWriteEnvelope.sources name ENTRY input VALUE snapshots. Declaration derivation computes a metered store-reachability closure adding eligible entry sources under ORIGINAL rigid type/kind/region/binder constraints. SAME Plan computes current-cell normal-exit relations back to those entry sources. Never interpret a source as a sequential poststate read.

At a call, evaluate all return/write substitutions from the SAME captured entry+OLD+lens state, consume original checked Move/reborrow exactly once, and apply poststate simultaneously. OLD is implicitly permitted in every write envelope. Child reborrows preserve original parents/dependencies and freeze the original mutable parent, with no duplication/reactivation.

Initial recursive, mutual-recursive and abstract calls use conservative canonical declaration envelopes including OLD. SAME universal Plan proves body effects refine them. No initial SCC summary precision solver, unchecked inferred canonical summary, or world-to-body callback is assumed. Private CarrierEffectProof is retained with exact successful VerifiedTemplateCfg source/CFG/analysis/world/ledger and records may effects plus MustOverwrite across ALL normal exits. Joins intersect must-overwrite facts. Divergence is not a fabricated write.

Precise selected-body refinement may be consumed only after the exact callee proofs required for that caller are available. A CALL strong replacement needs singleton Exact unique mutable destination AND sealed exact-body MustOverwrite. An &mut formal alone grants neither. Direct stores can establish their actual definite update locally. Valid programs requiring unavailable transitive precision are Unsupported, not Invalid.

Kind 115 is now a per-universal admission fact, not a root join. For one universal caller, it reads that caller's TEMPLATE_BODY/UNIVERSAL_ANALYZE proof plus only the universal callees required by reference-actual calls. It does NOT depend on closed caller ANALYZE or CONCRETE_PROOFS 114. Exact-body refinements are still keyed by the canonical provider declaration, exact original checked source/VerifiedTemplateCfg and world/ledger, and every use separately validates the selected target and certified substitution. Missing entries use the conservative declaration envelope without waiting on another body. Nongeneric Source bodies remain closed-only and supply no early summary; this does not silently add an extra universal pass. Universal body analysis itself always uses declaration envelopes, not another body's summary. Thus no caller-analysis-to-114 backedge, world callback or body-summary bootstrap cycle exists.

The publication seam is the following SAME ownership analysis API:

```rust
// Compiler-visible opaque capability, private fields and constructors:
struct CarrierEffectProof;
impl VerifiedTemplateCfg {
    fn carrier_effects(&self) -> Option<&Arc<CarrierEffectProof>>;
}
impl CarrierEffectTable {
    fn for_callees(templates: &[VerifiedTemplateCfg], budget: &PlanBudget)
        -> Result<Arc<Self>, String>;
    fn admit_reference_actual_calls(
        caller: &VerifiedTemplateCfg,
        templates: &[VerifiedTemplateCfg],
        budget: &PlanBudget,
    ) -> Result<(), String>;
}
fn analyze_closed_with_effects(
    closed: cfg::ClosedCfg,
    effects: &Arc<CarrierEffectTable>,
) -> Result<ConcreteCfg, Analysis>;
```

Each 115 fact validates one caller against the exact universal proofs it needs.
Provider/source/CFG/analysis/world/ledger identities are derived from those
proofs, not supplied raw keys or a caller success flag. Reject foreign bindings
and conflicting duplicate identities. Precharge duplicate scans, per-call
admission work and lookup work on the SAME ledger; no uncharged deep payload
clone. A proof retains exact source/CFG/analysis/facts, not its enclosing
VerifiedTemplateCfg. Closed lookup is private and validates the exact
BodySubstitutionProof/template, canonical target, certified substitution, facts
and phase bridge before using a refinement. The existing analyze_closed remains
the conservative no-summary path. Missing summaries do not grant family proof
or skip the normal lifetime/effect checks.

The existing exact verified-source capability retains universal carrier/effect analysis, not just header topology. Closed actual expansion, re-lowering using actual Copy/Move facts, SAME closed Plan and ConcreteCfg/native seals remain mandatory. World header topology never substitutes for body success. Both Source and Recipe require the SAME capability.

## 7. Ratified first admission and affine classifier

NEW ordinary reference actuals are fully EXPLICIT root reference chains ending in reference-free types. Existing reference-free and rigid-Param inference/forwarding remain. Newly inferred structural Ref actuals remain Unsupported until source-order inference/coercion semantics are independently frozen. The same counted carrier layout replaces old fixed depth arrays; no recipe-only depth permission exists, and no unchanged resource cap is raised.

After actual family expansion, classify OWNED transferred components by ownership Field/Index/Variant edges and STOP at each reference VALUE boundary. Referent/deeper positions are views of current pointee cells, not additional owned exits. Returning &(&mut U) copies only the outer shared handle if exact facts.must_copy proves it; the stored inner mutable value is not copied. Returning &mut(&mut U) transfers one owned affine outer handle. For writes, reroot at the WRITTEN VALUE cell, not the transient destination argument handle. Thus put<T>(&mut T,T) with T=&mut U has one written affine value.

Every Copy exemption uses SOLE facts.must_copy(component semantic Ty) in the exact proof context. Outer Copy does not prove inner mutable value Copy. Independently owned fields count separately. Alias deduplication requires canonical physical identity proof, never Ty/Param equality.

Initial affine admission is either <=1 affine returned owned value component with NO carrier writes, or <=1 affine written owned cell component with NO carrier return. Count all DISTINCT possible canonical destination cells after COMPLETE typed target expansion and proved alias merge. An affine write requires a SINGLETON EXACT unique mutable target. One formal with two alternative physical targets is not one destination and cannot weak-update one moved mutable value into both. Wider affine/result-plus-write correlation is Unsupported before reference-valued instance seal. Copy-only weak effects remain fully provenance/lifetime checked. OLD/new are alternatives in ONE cell, not independently owned handles.

## 8. All-body-safe compatibility and explicit boundaries

Old reference-free value proof is NOT family proof. Scan exact ORIGINAL source and ALL checked/dead metadata before publication. Any NEW ordinary structural-reference actual call or new carrier operation requires its common family/lifetime/effect proof even in an unused generic body or an outer instance whose own actuals are reference-free. No demanded-outer-argument fallback hides new operations.

Previously admitted bodies containing only old-domain operations may retain their exact old value proof without new family capability. Reference-valued instantiation of such a body is Unsupported until SAME exact body has universal family proof. No caller bool, default success, skipped body or recipe exception grants it. All source bodies remain in the all-body gate. Genuine invalid source is Invalid, unavailable family precision is Unsupported.

Explicit remaining stage boundaries: newly inferred structural-reference actuals; external source nominal reference actual syntax and explicit nominal lifetime/reference-field declarations not represented by current grammar; wider affine result/write correlation or multi-target affine writes; unknown cell aliases without finite COMPLETE ORIGINAL typed candidates; recursive precision needing pre-barrier MustOverwrite. Internal generic nominal products use the certified SAME interner path above, subject to full counted layout and universal/closed proof, not a general source fence lift. A derived nested call such as id::<Wrap<T>> becoming id::<Wrap<&U>> remains initially Unsupported at that new inner instance: instance_facts currently has no certified call-lineage input. Constructing a Wrap<&U> VALUE is distinct from granting an executable outer nominal-reference type actual. Existing original body and reference-free instances remain admitted. Later lineage extension must flow through canonical DirectCallProof/instance facts, not target shape alone.

## 9. Resources and freeze status

All symbolic rows, component walks, region constraints, family relations, concrete expansions, source evidence, proof retention, complete typed cell/candidate tables, origin memberships, liveness, snapshots, joins and old/new relocation peaks charge SAME PlanBudget and unchanged per-analysis caps BEFORE growth/copy/traversal. Resource failure is never semantic false or silent descriptor exclusion. Copy/Move, initialization, two-phase status, parent authority and canonical proof barriers remain the SAME solver.

## 10. Proof and pipeline integration

The existing source-to-native path stays authoritative. There is one world,
one exact facts service and ledger, one checked IR/CFG family, one ownership
transfer/worklist/join Plan and the existing native ConcreteCfg gate. No stock
compiler or external code-generation process participates in candidate builds.
Stock Rust is permitted only as the independent oracle and bootstrap tool.

Every source body, including unused bodies, is checked. Generic Source bodies
retain universal lowering/analysis. EVERY registered available Recipe body,
including zero-arity recipes, retains universal lowering/analysis. Nongeneric
Source bodies retain ordinary closed analysis. Required signatures and an
UnavailableProvided default are not fabricated executable bodies. Source
clone_from overrides remain ordinary globally checked Source providers.

World carrier views are declaration authority only. All original schemas and
source-only region metadata are checked before facts publication. Universal
family/effect success belongs to the exact VerifiedTemplateCfg. A value-only
compatibility proof cannot be reused as a reference-family capability. Facts,
structural interning, Copy evidence, header Clone membership or an available
recipe descriptor never supply an executable body proof.

The same exhaustive instantiation visitor retargets ALL source types and
metadata, including locals, expressions, places, patterns, call arguments,
canonical signatures, variants, dead calls and explicit borrow timing. It
retains immutable original-stage adjustment provenance. Fresh closed lowering
may select permitted concrete Copy/Move behavior but never reruns source
coercion or invents a mutable reborrow from the substituted Ty.

Demanded instances require exact closed structural validation and the SAME
closed Plan analysis. The root CONCRETE_PROOFS barrier still joins ALL sealed
plan analyses before any object request. OBJECT independently awaits that same
barrier before the sole emit_cfg_function entry. CHECK does not demand native
object emission. Recipe bodies remain demand-only native instances despite
all-available-recipe universal validation. No analysis-to-barrier cycle,
world-to-body callback or proof from an absent body is allowed.

Version native semantic/artifact identity for this integrated change as
jr-native-generic-carriers-v1, including canonical carrier/region/body-proof
semantics and explicit catalog origin encoding. Old private artifacts must not
be reused under changed semantics. Preserve source semantic paths, canonical
catalog identities, snapshot checks and fail-closed foreign-ledger rejection.
Do not use Debug text, numerical ID range or spelling as canonical authority.

## 11. Resource accounting and compatibility gates

The unchanged limits include 4096 combined concrete origins, 1,000,000
memberships, 64 normalized regions where region constraints are required,
shared depth budget 128 and the existing combined 128 MiB analysis-state bound.
Existing binder, IR/schema, work, node, byte and diagnostic caps also remain.
Section 4a preserves the qualified high-arity erased legacy path without
inventing aliases or giving new reference topology a missing-row bypass.

Symbolic families and their paths, cells and effect edges consume nodes, work
and bytes even when there are ZERO concrete LoanIds. The concrete-origin cap
alone cannot bound symbolic state. Preflight checked point counts multiplied
by fixed/family value slots, addressable cells and relation-word counts, plus
liveness, activation, worklist, dependency/ancestor closure and alias tables.
Count entry/OLD snapshots, target-lens traces, complete candidate-cover
validation, return/store reachability closure, source-by-destination relation
composition, each path/alternative and all old/new retained allocation peaks.

The same real allocation/sizing kernels must serve preflight and the Plan.
Concrete substitution recounts every actual typed component before allocation,
including proof/lineage retention and scratch. Cached hits validate authority
and depth and charge complete scans. No fresh budget per call, body, proof or
instance; no remaining-ledger-dependent availability downgrade. Exhaustion is
Resource, not NotProven, Invalid or an Unsupported admission miss. Unsupported
is reserved for an explicitly out-of-domain shape/precision operation, not a
way to hide a failed ledger charge.

Preserve all previously admitted reference-free source bytes and assertions.
Any old legal-unsupported source becoming admitted requires independent stock
and candidate workers 1/8 BUILD/CHECK/runtime proof before classification-only
promotion. Preserve its source, expected observation, stdout and hashes. No
negative test may be deleted or moved to an unrelated source. Valid programs
outside this stage are explicitly Unsupported, not advertised as invalid Rust.

## 12. Frozen exact-32 acceptance inventory

This inventory was provisional during design, then frozen and independently
stock-classified as recorded in section 15. It is not candidate native
acceptance. Exactly 16 intended native positives, 12 semantic invalids and 4
legal-excluded cases retain the complete frozen source/observation identities.
No postfreeze feature substitution, source repair or bucket change occurred.

### Intended positives

1. explicit_shared_root_actual_identity
2. rigid_parameter_forwarding_preserves_hidden_origins
3. shared_root_copy_without_referent_copy
4. single_affine_mutable_root_identity_transfer
5. internal_tuple_components_keep_distinct_occurrence_origins
6. internal_array_constant_index_component_transport
7. internal_nominal_variant_component_correspondence
8. partial_component_move_preserves_sibling_carriers
9. shared_carrier_put_replacement_preserves_incoming_origin
10. single_affine_put_preserves_checked_child_dependencies
11. current_cell_reload_and_return_follow_written_poststate
12. derived_aliases_observe_one_current_canonical_cell
13. separate_call_occurrences_keep_fresh_lifetime_bindings
14. shared_join_and_rebind_union_component_origins
15. firm_explicit_mutable_expectation_retains_checked_reborrow
16. recursive_shared_forwarding_uses_declared_envelope

P05-P08 concern internal components under admitted root-reference actuals, not
outer generic tuple/array/nominal-reference actual permission. P07 requires the
exact nominal/body-substitution witness and must not demand a derived inner
Nominal<Refs> executable actual. P11 uses Copy/shared carrier return or a scalar
observation, not the excluded affine-write plus carrier-return combination.
P12 ends the child access before resuming a unique parent. P15 must distinguish
original firm explicit reborrow from opaque forwarding whose Move is retained.
P16 uses only conservative declared-envelope precision, not pre-barrier callee
MustOverwrite. All observations are checked exactly once in source order.

### Intended invalids

17. local_referent_escapes_through_opaque_return
18. short_incoming_reference_escapes_external_put_cell
19. mutable_hidden_component_region_invariance_violation
20. same_erased_type_reuses_unrelated_call_lifetime
21. distinct_parameter_occurrences_cannot_exchange_return_authority
22. affine_root_actual_is_used_after_explicit_move
23. unbounded_opaque_parameter_is_duplicated_without_copy
24. indirect_put_through_shared_handle_is_invalid
25. live_derived_shared_alias_conflicts_with_mutable_cell_access
26. stored_reborrow_child_does_not_reactivate_parent
27. returned_written_cell_cannot_hide_short_incoming_origin
28. unused_generic_body_has_invalid_carrier_escape

N19/N20/N21/N27 must be genuinely invalid within the ratified source domain,
not Unsupported substitutes for missing permission checks. N26 includes the
outer unique-parent lifetime: reborrowing through an outer access of lifetime
'a cannot claim the stored inner lifetime 'b alone. The contrasting shared
inner Copy load must not inherit an artificial outer-handle dependency.
No inferred-reference or multi-affine exclusion may mask an intended semantic
negative. Unused-body checks remain real global publication requirements.

### Intended legal-excluded cases

29. newly_inferred_structural_reference_actual
30. multi_expanded_affine_result_components
31. mixed_affine_result_and_written_cell_flows
32. derived_internal_nominal_reference_actual_call

The final source inventory must instantiate the exact owned-value classifier,
not count nested pointee views as independently transferred handles. Additional
existing boundaries remain explicit in section 8 even if not a dedicated slot.
No extra corpus group or unreviewed source expansion is implied.

Before any source freeze or stock execution, X32 was explicitly corrected from
an unsubstantiated recursive-MustOverwrite witness to the already documented
derived inner nominal-reference actual boundary. No source, expected value,
bucket observation or receipt was rewritten. Recursive precision remains a
stage exclusion in section 8, not a claimed dedicated stock oracle case.

## 13. Independent oracle and implementation release gates

Before implementation, freeze every complete source, dictionary, identity,
category, scalar observation, literal stdout and hash in an authorized scratch
directory. A literal-only main wrapper may check run()->i32 and print only a
fixed success string, with every semantic observation evaluated once at its
original source point. No formatting, attribute or standard-library feature
may accidentally replace the feature under test.

Independent stock BUILD emits a linked artifact. Independent CHECK uses real
--emit=metadata only. A successful bin metadata CHECK requires the genuine
regular .rmeta artifact, which may be zero bytes; record actual path, size and
hash and exact argv. Do not silently replace it with metadata+link. Execute
BUILD artifacts for all legal cases and preserve exit/stdout/stderr. Invalid
cases must fail both phases without output artifacts. Preserve exact primary
diagnostic multiplicity, including genuine null-code messages. Stop at the
first source/category/runtime mismatch and preserve the attempt unchanged.
Any correction requires explicit root authorization and before/after receipts.

After a frozen stock inventory and full-file design review, commit/push the
contract, then issue separate reliable-file scoped implementation GO. Owners
must publish production plus ALL existing-fixture static compile-ready
checkpoints before adding at most THREE grouped regressions each. Interface
changes and fixture migrations are coordinated, never silently remapped.

Initial Rust/native execution remains HELD until root integrated-ready.
PHASE1 is root-only hosted library/all-target checking. PHASE2 releases owned
focused filters, with sole root strict-all-target lint and comprehensive gates.
Do not multiply redundant broad builds. Keep full failures visible. Root then
runs cargo fmt --check, hosted all-target check, strict clippy, complete old/new
tests, exact-32 candidate workers 1/8 BUILD/CHECK/runtime, proof-graph gates,
a real formatted example and tracked independent historical-corpus campaign.
Verify actual hosted location, investigate fallback, and preserve metered
remote tenant/billing isolation. Never install over stable justrust.

Record append-only FINDINGS, exact redacted run profiles and raw independently
verified corpus/graph/source hashes before claiming delivery. Commit/push each
working coordinated improvement and evidence. No speed, full-Rust or native
acceptance claim follows from stock classification or design feasibility.

## 14. Ownership during implementation, after scoped GO only

| Owner | Scope |
|---|---|
| frog | ir.rs, cfg.rs, shared data/substitution/evidence types and owned fixtures |
| goat | ownership.rs, borrow.rs, current cells, family flow, effects and sizing |
| wolf | traits.rs, traits/catalog.rs, world authority/regions/structural proofs |
| butterfly | frontend.rs, frontend/recipes.rs, source evidence/inference/provider integration |
| llama | NEW frozen oracle runner and NEW integration fixture only |
| root | mod.rs, instances.rs, graphs, documentation, evidence and final integration |
| reviewers | Read-only architecture/resource review, no source/build/probe activity |

Shared APIs come from the exact records above. New modules or scope expansion
need explicit coordination. Preserve configured Git identity and the current
requested clean-room branch. Do not create worktrees, install the candidate,
reset unrelated work, or merge private formats into stock tools.

The final provenance-bearing contract awaits its last whole-file review and
commit. It authorizes no project implementation, native probe or build on its
own. Stock classification is not candidate acceptance.

## 15. Frozen independent stock provenance

The scratch campaign froze EXACTLY 32 complete sources and dictionaries before
any stock invocation, with the explicit X32 prefreeze correction in section12.
No corpus existed before that correction. The final inventory remains
16 positives, 12 invalids and 4 legal-excluded cases. Original sources, names,
categories, expected integers, literal stdout and both source hash domains
were preserved byte-for-byte throughout the campaign. No postfreeze source,
bucket, expected-value or harness repair was required.

Scratch root: `/home/jeremy/.jcode/scratch/generic-carriers-oracle`.

| Artifact | SHA256 |
|---|---|
| `corpus.initial.json` and initial `corpus.json` | `e719eff4a15f64f0f7e575a82869c50ac3919e115a7d1a749204e1ef0b68fd56` |
| `initial.sha256.json` | `234a0e201f4c18ccdb553410383e26ad58e1d4e014d57c2276bd65a542537e33` |
| `attempt1/corpus.observed.json` | `400bf44235e1c5a34ff964a10ab61111696d7765010a3e66bd1d1bcf602f8842` |
| `attempt1/results.jsonl` | `878fcbff10a0c57a349013ff24733b0420fd889451218c7e8788af94c81661fa` |
| `attempt1/summary.json` | `e00bb8bebcf9d39db169b466e64419e6644588d5887f314141cb69c8e5ffeb18` |
| `attempt1/toolchain.json` | `a441b7dbecce095ff23cfa0c10d8c27dd3289861248eaed7c74ee79b7e2eb1ab` |
| `root-independent/summary.json` | `b6fc1d60be62198df3942b9cb2432fb57f2fd4239e643a184ced13de16a6a605` |
| `root-independent/receipts.jsonl` | `7c9f878e3cd6b2c61f4f3541e8c3eafacdc46a2b7c5178aeadafa5ba75304b01` |

The observed artifact adds exact stock metadata and changes only the original
`classification_observed` status from false to true. Root compared every other
original dictionary field against the frozen initial artifact and rehashed
every source/full_source. Original files and manifest remain intact.

Stock was `/usr/bin/rustc` 1.98.1, commit
`48a229ceaefd4985c50990b14116b6d856af0985`, target
`x86_64-unknown-linux-gnu`. Both the initial campaign and the separate root
recompilation used edition2024, JSON diagnostics and overflow checks, BUILD
`--emit=link`, and CHECK `--emit=metadata` ONLY. Genuine regular bin `.rmeta`
artifacts may be zero bytes; their actual sizes/hashes and exact argv were
retained. CHECK was never replaced by metadata-plus-link.

Each campaign passed all32 cases: 64 independent phase compilations and20
linked BUILD runtime executions, exact success literal stdout, empty runtime
stderr and exit0. The12 invalid cases failed BOTH phases with no output
artifacts and exact primary error multiplicities. N19, N21 and N26 have genuine
null-code `lifetime may not live long enough` diagnostics, preserved as such.
Root independently recompiled all32 from the frozen observed artifact and
compared diagnostic codes/messages/multiplicities and all legal runtime
observations. The root raw receipts also retain artifact sizes/hashes.

These are STOCK classifications and contract evidence ONLY. No candidate
compiler, family-flow patch, new implementation test, native acceptance,
performance improvement or unrestricted reference-generic support is claimed.
Project implementation still requires the contract commit plus separate
reliable-file scoped GO and all integration gates above.
