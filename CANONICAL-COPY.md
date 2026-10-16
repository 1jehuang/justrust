# Canonical Copy, Clone, and required-schema carrier topology

## Status and authorization

This is the frozen implementation contract after joint whole-document feasibility
clearance and independent classification of the exact32 stock corpus. A contract
commit is not source GO. Separate scoped owner authorization is required for
implementation or fixture generation, and separate integrated gates authorize
native builds and tests. No install or deployment is authorized. The previous nested-reference implementation and its independent
1084-case campaign are delivered. They establish fixed carrier-position
provenance, not permission to invent positions when an opaque type is substituted.

The next coordinated stage adds a common required-schema topology foundation and
uses it for trusted core traits. Source ownership, exact APIs, a fixed independent
stock corpus, and production plus existing-fixture checkpoints must be frozen
before source GO. Feasibility review is not patch or runtime acceptance.

The goal remains a complete independent Rust compiler. These are truthful first
client boundaries, not permanent architectural limits or claims of full Rust.

## External contract and non-goals

Preserve supported Rust semantics, diagnostics, source evaluation order, existing
manifests and source compatibility, canonical declaration identity, and every
existing correctness gate. No candidate user-program compilation may delegate to
rustc, Cargo, LLVM, an assembler, linker, interpreter, or stock executable.
Independent stock classification and compiler bootstrap are separate activities.

Canonical Copy permission must come from the canonical trait, never from a source
spelling, aggregate field shape alone, a generic type bit, or a backend shortcut.
Clone is an actual call whose body is checked. Copy does not imply that calling
Clone can be replaced by a bit copy. A manual Clone implementation on a Copy type
can observably change the returned value. Tuple and array Clone must invoke each
child Clone implementation, including children that are Copy.

Keep one world, registry snapshot, predicate engine, shared budget, ownership and
borrow Plan, instance frontier, proof barrier, and native backend. Do not add a
second checker, public proof boolean, caller-selected validation mode, special
Clone pointer operation, or name-triggered permission. Fail closed on authority
mismatch and fail honestly with legal Unsupported at an unimplemented boundary.

## Retained reference and ownership foundation

Retain the exact position-qualified records and services in
`NESTED-REFERENCES.md`, including `ReferencePosition { depth: u8 }`,
`InputReference { param, position }`, `InputRegion`, `ResultReference`, and the
positioned `RefContract`. Existing executable bodies support at most two shared
reference layers, or the existing single mutable layer. Nested mutable chains,
reference-containing aggregates, and depth greater than two remain Unsupported.
Ordinary source generic type actuals remain recursively reference-free.

Every actual carrier slot, typed formal position, call-result position, returned
origin, parent authority, and lifetime dependency must retain its identity. Inner
reference loads transport stored inner value origins. The transient outer read
requires authority, but does not add the outer handle's storage lifetime or its
loan as authority for the returned inner pointee.

The existing `ValidatedFacts::must_copy(&Ty) -> Result<bool, FactsError>` remains
the sole semantic Copy query. Its implementation proves the canonical Copy goal
in the same exact world, environment, phase, cache, and budget. Proved is true,
NotProven is false, and invalid, cycle, ambiguity, resource, or authority failures
remain typed errors. Schema facts cannot grant executable Copy permission.

Opaque generic parameters remain opaque move-shape leaves, even with Copy proof.
Explicit `Operand::Move` still consumes a Copy value. Return initialization
inspection remains an initialization read, not an implicit Copy obligation.
Projection, alias, field, layout, and mutation permissions do not follow from Copy.

## Immutable trusted catalog and declaration identity

One private catalog builder allocates canonical core identities before source
registration. Its agreed lifecycle is:

```rust
CoreCatalogBuilder::new(snapshot: SnapshotId, budget: &PlanBudget)
    -> Result<Self, FactsError>
base_declarations(&self) -> &[CatalogDecl]
source_start(&self) -> u32
finish(self, source_end: u32, inventory: RecipeShapeInventory, budget: &PlanBudget)
    -> Result<Arc<CoreCatalog>, FactsError>
```

Base canonical declaration IDs occupy a prefix. All source declarations follow.
After all source IDs are fixed, sorted and deduplicated finite recipe descriptors
occupy the suffix. Shape inventory examines raw source schemas and AST, including
unused and dead declarations. It cannot query body facts, proof results, or a
partially published world. No runtime arbitrary declaration append is allowed.

`SchemaCx::core_catalog() -> &Arc<CoreCatalog>` supplies the exact immutable final
catalog to world construction. Registry, catalog, and world must agree on snapshot,
version, ledger, reserved IDs, and declaration origins before any publication.
The raw adapter retains no world or facts callback and creates no ownership cycle.

Semantic keys include `DeclOrigin::CoreCatalog { version, item/recipe }` and the
stable descriptor, not a source path spelling or accidental numeric ID. Source
origins remain distinct. Private catalog construction, not caller-provided header
metadata, is the authority for builtin rows and proof recipes.

Trusted public paths resolve to the same IDs: core/std `marker::Copy`,
`marker::Sized`, `clone::Clone`, and `ops::Drop`. Normal lexical and explicit import
resolution precedes shadowable prelude fallback. A local trait named Copy, Clone,
Sized, or Drop is an ordinary source trait and grants none of the canonical rules.
All semantic comparisons use canonical identity, not strings.

The finite catalog includes canonical Sized, Clone and its clone and clone_from
items, Copy, and Drop with its required drop item. Drop::drop has the canonical
`fn drop(&mut self) -> ()` schema, borrowed-mutable receiver, positioned input
region metadata, and no result-reference rows. Its required declaration has
default None and no callable provider, not UnavailableProvided. Malformed Drop
coverage, duplicates, ownership, receiver, or signature remain semantic invalid
before the well-formed Drop-only execution boundary. Additional catalog entries
require an explicit contract extension, deterministic key versioning, and
correctness evidence.

Version the integrated instance/artifact semantic key for canonical trait
permission and signature-origin topology. Encode catalog origin and descriptor
canonically. Never reuse older source symbols or artifacts solely because their
ordinary source path and concrete type arguments look unchanged.

The one traits-owned origin representation is
`DeclOrigin::{Source, CoreCatalog { version: u32, item: CatalogItem }}`.
`CatalogItem` has Sized, Clone, CloneItem, CloneFromItem, CloneFromDefault, Copy,
Drop, DropItem, and Recipe(RecipeDescriptor) variants. Recipe uses the same typed
descriptor as the provider, and identifies only an admitted executable body.
`PlanCtx::declaration_origin(id: DeclId) -> Result<traits::DeclOrigin, String>`
reads the original source registry or exact immutable catalog record. The instance
encoder queries origin first, encodes explicit stable discriminants, version and
descriptor fields for catalog entries, and uses existing semantic_path only for
Source. It never serializes Rust enum layout, Debug output, paths, or local IDs as
catalog authority. No duplicate origin field is added to semantic_path.

## General supertraits and implementation validity

Add real `TraitHeader.supertraits: Arc<[TraitRef]>`. This is a general relation,
not hardcoded permission in the parser or ownership solver. Canonical Copy has
Clone as a supertrait. Canonical Clone has Sized as a supertrait. Assumption
closure is transitive, deterministic, metered, and capped at 256 retained entries
before growth. Repeated or cyclic metadata cannot loop or silently grant evidence.

Ordinary source generic type parameters receive their language-mandated canonical
Sized predicate from their declaration. Trait Self does not acquire Sized merely
because it is opaque. A genuine canonical supertrait or declared predicate can
prove it. Unsupported unsized source forms remain explicit boundaries.

Build immutable complete headers first. Check structural headers, independent
original schema agreement, conformance, ownership/coherence, and supertrait
requirements before publishing facts or native work. Implementation checking uses
a private predicate environment containing ONLY its declared bounds and their
validated supertrait closure. It must not assume the very Self:Copy or Self:Clone
fact whose implementation validity is being established.

A source canonical Copy implementation requires independently provable Self:Clone,
canonical Sized, Copy for every owned field in every variant, and no canonical
Drop implementation. Array element Copy is required even for length zero. A
nominal type with all Copy fields but no canonical Copy implementation is not Copy.
Candidate proof construction carries these well-formedness obligations so an
invalid candidate cannot prove another header before its own validation finishes.

Header evidence for Clone is not proof that its body is correct. Every actual
source implementation body, including unused ones, must pass the same universal
and closed checks. A mutually useful Clone body using `*self` and a valid Copy
implementation is not rejected by an invented body-proof/header-proof cycle.

Source canonical foreign-trait implementations obey ownership and coherence.
Within the supported bounded source type forms, explicit canonical Copy and Clone
implementations require local nominal Self. Primitive, tuple, array, and shared
reference builtin implementations cannot be forged or overridden by source rows.
Existing local source traits retain their admitted implementation shapes.
Explicit source implementations of canonical Sized are semantic invalid Rust.
Canonical Drop implementations require a legal local nominal target.

Inventory all canonical Drop heads before deciding Copy validity. Copy plus a
canonical Drop implementation is semantic invalid Copy, not a legal-Unsupported
substitute. A well-formed legal Drop-only program is Unsupported before publication
until destructor execution is implemented. No source destructor may be accepted
and silently omitted. A local trait named Drop does not trigger this rule.

## Builtin logical evidence is not method execution

Primitive, unit, tuple, array, and shared-reference Copy/Clone evidence is provided
by private catalog rules in the same conditional proof DFS and cache. Tuple and
array Copy require component Copy, including zero-length arrays. Tuple and array
Clone require component Clone, not component Copy. Shared references are Copy and
Clone without requiring their referents to be Copy or Clone. Mutable references
are never Copy. Sized uses a trusted structural rule, not source impl rows.

Logical trait membership must not instantiate or compile methods. In particular,
Copy and Clone on `&&T` can be proved even though invoking clone for Self = `&&T`
would require a three-layer receiver `&&&T`, beyond current executable support.
The trait goal succeeds. That method dispatch is Unsupported before the frontier
or native seal. Do not regress intrinsic shared-carrier Copy to avoid this split.

Never-type logical or private recipe metadata does not imply source admission of
unstable `!` positions or an executable CloneNever body. Such execution needs
independent stock evidence and a genuine analyzed body. Otherwise dispatch is
explicitly Unsupported, without affecting unrelated trait evidence.

## Why fixed contracts do not authorize reference-valued Self

The canonical required declaration `clone(&Self) -> Self` has an opaque result.
Its rigid fixed contract has no result reference row. Substituting Self = `&T`
creates a second input position and a result reference position. The already
analyzed fixed recipe `&&T -> &T` proves the implementation's behavior, but does
not by itself authorize new canonical trait-item origin channels.

Do not solve this by rerunning concrete candidate derivation, relaxing
`same_regions`, allowing reference actuals globally, matching Clone spelling, or
reusing an opaque executable body's proof. A common declaration-bound residual
schema must state where future carrier positions can come from.

## Exact common signature-origin schema

The agreed data-only IR records are:

```rust
struct ParameterCarrier {
    parameter: ParamId,
    relative: ReferencePosition,
}
struct InputCarrierTail {
    param: usize,
    prefix: u8,
    parameter: ParamId,
}
struct ResultCarrierTail {
    prefix: u8,
    parameter: ParamId,
    inputs: Arc<[InputCarrierTail]>,
}
struct SignatureOriginSchema {
    fixed: Arc<RefContract>,
    input_tails: Arc<[InputCarrierTail]>,
    result_tails: Arc<[ResultCarrierTail]>,
}
```

The single helper is:

```rust
ir::derive_signature_origin_schema(
    signature: &Signature,
    env: &GenericEnv,
    budget: &PlanBudget,
) -> Result<Arc<SignatureOriginSchema>, String>
```

It is data derivation, not proof and not a facts factory. `GenericSchema` remains
unchanged. World construction derives and retains an immutable side-table keyed
by the authoritative declaration ID and exact canonical original schema and
GenericEnv from `SchemaCx::declaration_schema`. Compare original raw declaration
schemas independently from HeaderSet before accepting any tails.

Walk only root leading reference chains and their terminal owner-qualified Param.
Validate exact binder membership. Record parameter index, original reference
prefix depth, and ParamId. Borrow kinds and occurrence locations come from the
canonical original signature, not a caller claim. A root opaque value has prefix
zero. There are no carrier tails inside aggregates in this first client.

Each result tail captures ALL original input tails of the EXACT SAME ParamId in
deterministic original order. Distinct T and U do not share tails even when actual
types later coincide. No body inference, concrete type-equality search, method
name, or recipe identity can introduce another input candidate.

Fixed existing rows and candidate payloads are transported unchanged except for
a certified canonical region renumbering. They are not rederived after substitution.
The residual tail schema is immutable declaration authority, not an invitation to
search newly concrete layout for origin paths.

## World-only realization and certificate

The private world records are:

```rust
struct CarrierRegionBinding {
    carrier: ParameterCarrier,
    region: RegionParamId,
    inputs: Arc<[InputReference]>,
    results: Arc<[ReferencePosition]>,
}
struct SignatureTopologyProof {
    world: WorldId,
    registry: SnapshotId,
    environment: usize,
    phase: FactsPhase,
    declaration: DeclId,
    arguments: Arc<[Ty]>,
    schema: Arc<SignatureOriginSchema>,
    fixed_regions: Arc<[RegionParamId]>,
    carriers: Arc<[CarrierRegionBinding]>,
    realized: Arc<RefContract>,
    budget: PlanBudget,
}
```

Only the world constructs this certificate after canonical declaration, binder,
argument, phase, world, snapshot, catalog, and same-ledger agreement. It has no
Arc back-reference to the world. The raw SchemaCx structural substitute is an
unsealed intermediate and cannot mint the certificate or callable facts.

`environment` is an index into the SAME world's immutable PredicateEnv table,
which retains exact ordered GenericEnv scopes, sealed assumptions, declaration
context, and binder membership. Validate world, snapshot, index, caller parameter
membership, phase, and ledger before lookup or certificate/cache reuse. The
existing world-owned EMPTY context uses usize::MAX and must be handled separately:
only its genuine empty environment and assumptions qualify. Never use that empty
sentinel for private header contexts with declared bounds.

Register uniquely indexed private declared-bound environments after structural
header validation and before semantic Copy WF or topology conformance. These
unpublished contexts contain declared bounds and validated supertrait closure,
NOT later callable-body implied Self assumptions. No public facts are published
until all header and implementation checks succeed. This preserves one engine
without a bootstrap empty-context or body-assumption authority leak.

`fixed_regions` maps every original canonical region ID to the realized canonical
ID. Latent regions are keyed by `ParameterCarrier(ParamId, relative depth)`, are
disjoint from declared region symbols, and are shared only between occurrences of
the SAME original parameter. Unique sorted carrier bindings map exact checked
prefix-shifted input and result positions. Preserve all original region relations.
Add required nested inner:outer WF edges before full closure and canonical
renumbering, retaining the complete old-to-new correspondence.

Introduced result rows use the appropriate input-envelope variant over the frozen
same-Param shifted anchors. Do not search for coincident concrete referent types.
Validate every position, kind, region, candidate, and dependency, including dead
metadata, under the same budget before publication.

Realization belongs inside the existing world trait-signature instantiation path,
BEFORE executable shape checks, conformance, and existing return refinement.
The public `instantiate_trait_signature` still returns Signature. Private internal
results retain the topology proof and compose it into the existing immutable
ReturnRefinementProof. Canonicalization validates the original TraitItem subject,
then atomically replaces callee AND signature with the selected canonical provider
result. A subsequent Direct check requires that exact provider schema.

For Self = `&T`, `clone(&Self) -> Self` has input tail `(0, 1, Self)` and result
tail `(0, Self)`. Realization yields input0 position1 and result position0 sharing
hidden `(Self, relative0)`. The new result envelope contains ONLY input0 position1.
The original outer receiver position0 remains a transient access loan, never the
returned pointee's origin or lifetime authority.

OPEN Self = opaque ParamT keeps its opaque result, fixed rows, and TraitItem call
plus world-owned residual schema. There is no executable phantom carrier slot.
Distinguish structurally known OPEN Self = &ParamT: with ordinary executable T
still recursively reference-free, its known reference topology CAN be realized
under exact Universal caller context. The callee may remain abstract TraitItem
with fixed &&T -> &T rows. No conditional source impl is selected from an opaque
assumption. The same Plan checks actual inner position1 provenance.

For example, a generic keep(x: &T) -> &T can call canonical Clone with Self = &T
without ever passing a reference actual to executable keep. Later keep::<S>, with
reference-free S, transports those rows, closes Self = &S, and selects the fixed
SharedRef recipe<S> through the same atomic canonicalization and closed proof.
Both certificate and cache include exact caller environment and phase. This is
known constructor topology, not a general symbolic executable carrier bundle.
Fully closed authorized RefSelf realization is unchanged. Ordinary source generic
actuals do not acquire reference permission.

## First-client authority boundary

Latent topology realization initially applies ONLY to NONEXECUTABLE trait-item
declaration schemas, required OR provided. A provided signature is not its body;
it remains unavailable unless a real supported provider exists. Every selected
source or recipe executable schema must independently be
fixed-positioned, with recursively reference-free remaining generic actuals, and
must pass the existing same-Plan universal and closed body proofs.

Reference-valued executable opaque parameters, generic reference-actual wrappers,
and executable default providers with latent bundles remain Unsupported until a
genuine symbolic carrier-bundle flow extension exists in the SAME Plan. No claim
is made that an old opaque universal proof covers those values.

Trait Self role is derived from authoritative binder ownership and canonical
registry/catalog metadata. No caller bool, source spelling, ordinary generic type
argument, or public validation skip can select this role. The compiler-private
bridge is:

```rust
ir::substitute_schema_ty(
    ty: &Ty,
    env: &GenericEnv,
    args: &[Ty],
    caller: FactsCtx<'_>,
    cx: &mut impl SubstitutionCx,
    registry: &dyn traits::SchemaCx,
    budget: &PlanBudget,
) -> Result<Ty, String>
```

It shares the private structural substitution walker. Existing declaration_info
and declaration_schema are sufficient, with no role flag or new metadata getter.
The world verifies exact nonexecuting TraitItem ownership, its Trait declaration,
Self binder index0, ordered membership, original canonical schema, and same ledger
before the adapter uses this bridge. Required or provided declaration metadata
can be structurally substituted, but this does not authorize a provider body or
method execution. A provided mixed-mutable schema may remain Unsupported on use.
The returned raw type/signature is UNSEALED until world topology proof and ordinary
shape/refinement checks finish. Existing substitute_ty and instantiate_checked
retain their recursively reference-free executable-actual fence. No source body
or recipe uses this bridge to evade that fence.

## Ordered receiver lookup and reference-valued Self syntax

Use ONE readonly bounded receiver selector for both scalar hints and the actual
checker. For each supported builtin autoderef level R, consider candidates R, &R,
and &mut R in that order, then continue to the next dereferenced R. Match the
canonical receiver FORMAL to the candidate to infer Self. Inherent methods have
priority WITHIN a candidate, then visible scoped trait implementations. Do not
greedily strip one reference or choose Self equal to the receiver value type.

For r: &S with S: Clone, candidate &S matches clone with Self = S and returns S.
Only genuine inapplicability or NotProven may advance to &&S matching Self = &S
and returning &S. Resource, cycle, ambiguity, invalid metadata, and authority
errors propagate, never act as fallback. Once a logical candidate is selected,
an executable depth or provider Unsupported cannot fall back to a different Self.

Retain the exact selected adjustment: builtin deref depth and Identity, SharedAutoref,
or MutAutoref. Selection and hints must not evaluate or move the receiver. Apply
the chosen adjustment exactly once, preserve receiver-before-arguments order, and
retain current genuine Call carrier restrictions. No repeated source evaluation,
eager non-Copy owner move, or unconditional added Deref is permitted.

Identity means no newly added autoref shape, NOT permission to move an existing
mutable carrier used by a borrowed-mut receiver. That case emits a genuine
implicit mutable reborrow with the existing exact TwoPhase protocol. By-value
receivers still consume normally. Explicit source borrows remain Immediate, and
all candidate-specific authority, activation, and lifetime dependency rules stay.

Reference-valued Self syntax is admitted in authoritative trait Self roles, such
as `<&T as Trait>::item` and trait-call Self inference, using exact trait owner and
Self binder index0. It does not admit `f::<&T>` or reference actuals to ordinary
function, method, or nominal type parameters. It does not waive foreign-trait
coherence or expand arbitrary autoderef/owned-temporary borrowing scope.

## Genuine executable recipes and body publication

Known logical recipe descriptors include CloneInt(IntTy), CloneBool, CloneUnit,
CloneNever, CloneTuple { arity }, CloneArray { len }, and CloneSharedRef. Available
body-backed descriptors alone receive executable recipe declarations. Such a
recipe has a deterministic schema, origin, and provider identity. CloneNever has
no executable registration in this first client.

Keep ALL raw shape inventory separate from executable declaration registration.
A source tuple with arity33 or an array of length4096 may be admitted source even
though the current generic or unrolled Clone body exceeds an existing body limit.
Merely mentioning an unused such shape must not poison an unrelated program by
eagerly registering an impossible executable recipe. Logical trait evidence is
independent from that recipe's executable availability.

Use ONE recipe structural traversal with counting and emitting sinks. Counts come
from the SAME events that generate actual fixed recipe IR, not a copied frontend
heuristic or new arbitrary array cutoff. Checked upper bounds cover locals,
blocks, points, typed carrier slots, loan sites, typed formal positions, typed
call-result positions, move paths, operations, and memberships. Include resolved
target records, projections, candidate-specific authority members, parent and
lifetime dependency closure memberships, Paths retention, and verification scratch.
Extract the existing borrow Plan and ownership state sizing kernels, target payload
and old/new growth kernels, and Paths retention kernels and reuse them in BOTH
real analysis and inventory admission. The single data-only admission entry is:

```rust
ownership::preflight_recipe_counts(
    counts: &RecipePlanCounts,
    budget: &PlanBudget,
) -> Result<RecipeAdmission, FactsError>
// RecipeAdmission::Available(footprint) | IntrinsicLimit(reason)
```

Descriptor availability compares its intrinsic universal-body footprint against
the unchanged ABSOLUTE limits: combined32 binders, existing schema/IR depth and
node limits, combined4096 origins, one-million memberships, and the SUM of
ownership plus borrow state peaks under the existing128MiB ceiling. Include
retained headers, masks, snapshots, worklists, scratch, and relocation peaks. A
loan count must include formal and call-result positions, not only issued loans.
For example,4096 unrolled element borrows plus one formal already exceed4096.

An intrinsic limit or checked count overflow retains the raw logical descriptor
and its deterministic rejection reason, but allocates NO executable recipe DeclId
or provider. It is NOT UnavailableProvided and NOT logical NotProven. Actual
method demand reports that precise resource or Unsupported boundary BEFORE any
Direct target or frontier entry. A missing uninventoried shape is an authority
error, never permission for runtime catalog growth.

Admission is NEVER selected by the snapshot's remaining budget. Inventory,
counting, and preflight work use the SAME ledger; current work/byte/allocation
exhaustion is a WHOLE planning Resource error, not a reason to silently omit a
descriptor. Do not reserve an unbuilt body's whole footprint or create another
budget. Available registered recipes still ALL pass real universal body analysis.
Actual emitted counts must match or be bounded by the admitted footprint, or fail
as an internal error. Demanded concrete type shapes rerun all actual resource and
proof checks. This is not a body-proof callback in catalog construction.

The agreed provider entry is:

```rust
PlanCtx::checked_body(
    target: &CallTarget,
    facts: &ValidatedFacts,
    budget: &PlanBudget,
) -> Result<CheckedFunction, String>
```

Source and recipe providers produce actual checked IR through canonical call
closure, lower again after substitution, run the same ownership/borrow analysis,
and retain exact ConcreteCfg authority. No direct backend intrinsic, fabricated
analysis proof, name stub, or native pointer relabel is permitted.

A shared-reference recipe first supports Self = `&T` with reference-free T and a
fixed `&&T -> &T` schema. Its actual inner load must prove the stored inner origin
and permit the result to survive a temporary outer handle while the original
referent lives. The converse dangling-referent case must fail.

Every source body and source impl body remains globally checked even when unused.
Available catalog recipes are ordinary checked bodies. Required non-body schemas
are not body roots. The canonical provided clone_from default retains identity and
schema but cannot poison unrelated builds merely because its unavailable body is
listed in the catalog. Demand for that unavailable body must fail Unsupported
before instance/native publication. A real supported source override is an
ordinary source body, globally checked and dispatched normally.

The single shared metadata enum is defined in traits and reexported by frontend:

```rust
enum CallableProvider {
    Source { source_index: usize },
    Recipe { descriptor: traits::RecipeDescriptor },
    UnavailableProvided { declaration: DeclId },
}
// Field on immutable trusted CatalogDecl:
callable_provider: Option<CallableProvider>
CoreCatalog::declaration(id: DeclId) -> Option<&CatalogDecl>

PlanCtx::callable_provider(decl: DeclId) -> Result<CallableProvider, String>
PlanCtx::from_shared_program(program: Arc<Program>, budget: &PlanBudget)
    -> Result<Self, String>
```

There is no second frontend provider enum, SchemaDecl provider field, or new
SchemaCx callback. The immutable trusted CatalogDecl carries Recipe,
UnavailableProvided, or None, NEVER Source. Its readonly lookup is supplied
through the existing exact SchemaCx::core_catalog identity. The world validates
descriptor, origin, and canonical schema agreement against that SAME catalog.
World callable_facts, instance_facts, discharge, and resolution reject an
unavailable catalog provider before returning a Direct target or frontier entry.
Signature-only catalog declarations cannot become Direct body targets. Late
checked_body rejection alone is insufficient.

PlanCtx::callable_provider reads that SAME catalog record or the authoritative
source index, not an independent availability table. Actual source overrides use
validated source headers and the all-source body gate, and are not rejected by
provided item identity. Required non-body schemas remain outside body enumeration.

Source index is a lookup into the existing Free/Associated source table, never a
semantic identity. The root RESOLVE publishes Arc<Program>. SIGNATURES and PlanCtx
retain that EXACT Arc with no source rewrite or per-instance AST clone. Existing
from_program(&Program, budget) is a compatibility wrapper that must preflight
same-ledger work and peak retained bytes before its single ownership clone, then
delegate. The production root uses from_shared_program.

Existing callable_count/decl/index enumerate EVERY actual source body and ALL
finite registered AVAILABLE body-backed recipes, including unused recipes. They
exclude required non-body schemas and UnavailableProvided defaults. CloneNever
logical metadata has no executable recipe registration without genuine body and
stock evidence, and is not mislabeled UnavailableProvided.

TEMPLATE_BODY requests checked_body with Universal facts and a symbolic CallTarget
whose ordered arguments are the exact environment's rigid parameters. It publishes
only the canonicalized checked result. Recipe bodies ALL receive lower_template
and verify_template, including zero-arity recipes; existing Universal wrappers
already support an empty generic environment. Source generic bodies keep their
universal gate and all source nongeneric bodies retain concrete checking.

Concrete entry seeding retains ordinary source nongeneric roots but recipes are
instantiated only when demanded. Available recipe universal proofs still precede
MONO_PLAN. Demanded concrete recipes instantiate the verified template, re-lower,
close, and analyze with exact Closed facts. checked_body is Universal-only and is
not a separate closed re-type path. No per-provider proof shortcut exists.

The catalog-owned shared-reference blanket matches through kind-checked recursive
Ref matching in the SAME proof engine. Its logical referent parameter has the
canonical ?Sized role and can match a reference without instantiating any recipe.
The selected executable CloneSharedRef recipe has a distinct ordinary implicit
Sized, recursively reference-free remaining T parameter. Source explicit canonical
Copy/Clone for &T remains invalid coherence. The declaration role bridge is not
an ownership waiver or an executable parameter exemption.

Retain universal source validity before MONO_PLAN, and the cached CONCRETE_PROOFS
barrier before BUILD requests any OBJECT. OBJECT itself awaits the same barrier
before the sole emit_cfg_function route. A requested unavailable provider cannot
be a sealed native entry. No analysis-to-barrier cycle is allowed.

## Resource, cache, and diagnostic contract

Use the same PlanBudget for catalog inventory, header closure, Copy WF, logical
proof, tail derivation, topology expansion, canonicalization, provider body
construction, instance planning, analysis, and native output. Do not reset a cap,
create an independent budget, or allocate before precharge.

Retain the existing two-reference-layer executable boundary, 64 positioned regions,
4096 combined origins, one-million memberships, shared proof cache limits, and
64KiB diagnostic allowance. Check prefix plus actual depth and nested mutable
boundaries BEFORE expansion growth. Logical membership does not allocate a method
signature merely to test an executable limit.

Charge complete raw schema comparison, ordered binder equality, retained tail and
candidate arrays, full region closure, certificate maps, topology cache keys,
recipe instructions, and every relocation old/new peak before work or allocation.
Count every type-derived carrier/formal/call-result position, including legacy
zero-row contracts and dead calls. Preserve current high-arity compatibility.

Cache identity includes exact world/snapshot/catalog version, declaration and
original owner-qualified environment, ordered actuals, phase, canonical contract,
position/type-use context, residual schema identity, and ledger authority as
appropriate. No cache reuse by a bare concrete Ty or method name. Failed, cyclic,
NotProven, and resource outcomes retain distinct semantics. No lock is held across
recursive proof or provider callbacks.

Diagnostics remain bounded and useful, errors and failing tests in full. Retain
the documented fixed source-independent exhausted legacy String transport
exception. Never format rich errors into an uncharged intermediate string.

## Exact frozen source inventory

Exactly32 cases were proposed and then independently frozen and stock-classified:
16 positive,12 invalid,4 legal-excluded. Their unchanged names and categories are
listed below. The initial design list itself did not authorize source generation
or probes. The separately authorized immutable artifacts and independent stock
receipts are recorded at the end of this contract, not claimed as native evidence.

Positive16:

1. canonical_core_std_identity_and_shadowable_fake_names
2. true_supertrait_closure_and_default_sized_generic
3. manual_copy_clone_pair_body_uses_copy_without_header_cycle
4. conditional_nominal_copy_uses_only_declared_field_bounds
5. shared_reference_copy_without_referent_copy_or_clone
6. primitive_unit_and_empty_product_clone_recipes
7. manual_clone_on_copy_nominal_preserves_observable_behavior
8. tuple_clone_preserves_nontrivial_manual_copy_child_clone
9. array_clone_preserves_nontrivial_manual_copy_child_clone
10. shared_reference_clone_inner_escapes_outer_handle
11. receiver_candidate_pointee_clone_precedes_reference_clone
12. receiver_candidate_shared_reference_fallback_and_inherent_priority
13. real_clone_from_override_runs_while_default_stays_unavailable
14. logical_copy_clone_double_shared_reference_without_method_execution
15. T_U_coincidence_preserves_declared_clone_result_origins
16. unused_nominal_field_tuple33_array4096_does_not_poison_scalar_main

Invalid12:

17. fake_copy_spelling_does_not_grant_second_owned_use
18. copy_shaped_nominal_without_copy_impl_remains_move_only
19. conditional_copy_impl_missing_required_field_bound
20. copy_impl_without_clone_supertrait_is_invalid
21. unused_bad_clone_body_moves_noncopy_through_shared_self
22. canonical_copy_with_drop_is_invalid_before_unsupported
23. foreign_canonical_copy_or_clone_impl_is_orphan_invalid
24. explicit_canonical_sized_impl_is_invalid
25. mutable_reference_does_not_gain_copy_permission
26. shared_reference_clone_result_escapes_original_referent
27. wrong_T_U_clone_result_region_despite_concrete_coincidence
28. zero_length_array_field_still_requires_element_copy

Legal-excluded4:

29. ordinary_reference_generic_actual_clone_wrapper
30. unavailable_canonical_clone_from_default_demand
31. forced_clone_on_double_shared_reference_requires_depth3_receiver
32. legal_canonical_drop_only_requires_destructor_execution

P14 uses existing concrete &&i32 carrier reuse. Its canonical Copy proof includes
the real Clone supertrait obligation, without demanding a clone method, ordinary
reference generic actual, or a new local trait structural-Self impl grammar solely
for a witness. An internal world group separately checks the explicit Clone goal.
P15/N27 use reference-free ordinary T/U actuals, known trait Self=&Param topology,
distinct declared regions, and exact original carrier authority. P16's large
shapes are fields of an uninstantiated nominal type, not a demand for their Clone
bodies. Product child-Clone observations must be independently classified. A stock
discrepancy requires an explicit contract decision, never silent output repair.

## Verification and freeze procedure

Coordinated source ownership after a separately committed contract and explicit
scoped GO is: frog owns ir.rs/cfg.rs, goat owns ownership.rs/borrow.rs, wolf owns
traits.rs plus new traits/catalog.rs, and butterfly owns frontend.rs plus new
frontend/recipes.rs. The new files are nested modules declared by their owning
parent file, not independent root-module changes. Root owns compiler graph and
instance integration, semantic key encoding, independent graph tests, example,
documentation, campaign and evidence. Llama owns only new
experiments/canonical_copy_correctness.py and tests/canonical_copy.rs from the
unchanged frozen32 dictionaries. Reviewers remain read-only.

Synthetic fixtures use the exact production catalog-prefix layout. The common
cfg(test) helper `test_support::source_decl(snapshot, ordinal, budget)
-> Result<DeclId, FactsError>` uses the same checked source_start layout kernel as
the catalog builder. Existing world/facts_for_signature helper signatures remain
unchanged. Owners migrate their supplied source IDs and matching binder, nominal,
variant and callee references consistently. No caller ID or proof subject is
silently remapped or aliased, and reserved-prefix collisions remain invalid.

Before implementation, jointly close the remaining lifecycle questions, read this
ENTIRE document, record feasibility or concrete blockers, and agree one API. Then
freeze exactly32 names/categories, generate only an authorized scratch oracle,
stock BUILD plus genuine metadata-only CHECK, and exact valid runtime outputs.
Preserve initial sources and all raw receipts. Code-less lifetime errors require
exact structured level/message checks, not invented diagnostic codes. Root
independently reclassifies the frozen sources before committing the contract.

Only after contract commit and separate scoped owner GO may production be edited.
Each owner must publish production plus ALL existing owned-fixture compile-ready
checkpoint before adding at most the approved grouped regressions. No owner Rust
or native execution before the root integrated gate. First root check library,
then all targets, then explicitly release scoped owner tests. Root owns broad
suite, strict alltarget lint, graphs, bootstrap, frozen worker1/8 oracle, historical
promotions, and independent tracked campaign.

Prove canonical identity and shadowing, supertrait closure, Copy WF and unused body
checks, nontrivial manual Clone inside Copy products, shared carrier Copy without
referent bounds, genuine shared-reference Clone provenance, fixed topology
transport versus T/U coincidence, provided override/default boundaries, explicit
Move consumption, canonical Drop ordering, foreign/world/ledger denial, cycles,
dense resources, dead metadata, and object barrier ordering. Keep semantic-invalid
separate from legal-Unsupported. Do not mutate frozen source bytes to get a pass.

Promote an old boundary only after independent unchanged-source stock and native
worker1/8 BUILD/CHECK/runtime proof. Preserve all other dictionary/source/stdout
bytes, retain raw receipts, and state exact classification-only changes.

Use hosted justrust checks/tests/lints, record actual location and fallback, retain
redacted cited run summaries, cargo fmt, and scoped diff checks. Bootstrap the
candidate without installing it. Re-run all tracked independent corpora with one
identified binary, parse every raw result and graph, run a real independent example,
append FINDINGS without rewriting its prefix, commit and push implementation and
evidence separately. Never claim speed, default promotion, or full Rust from
functional correctness gates.

## Frozen stock oracle and independent receipts

The exact32 inventory above is now frozen with16 positive,12 invalid,4 legal
excluded cases. The original sources, names, categories, integer observations,
literal stdout and all declaration/run and full-wrapper hashes were frozen
before any stock invocation. No source, expected value, category or dictionary
repair was needed. P08's nontrivial manual tuple child clones observed203 and
P09's array child clones observed405, exactly their initial hypotheses.

The immutable scratch artifacts are under canonical-copy-oracle:

- corpus.initial.json SHA256
  `3438c97aeb77cb6e84a7f6a5aaba6eacbb5691336befdd5287bc3f77a2841780`.
- attempt2/corpus.observed.json SHA256
  `3c3759705d094af4f4c53a53e8bcb3bc400da7e1d31a84e878f5e2708502b1d7`.
- attempt2/results.jsonl SHA256
  `8e44bae83cba3499bfdb9e0f952fe97fbb86c61b341955df902782e27d41898e`.

The stock compiler was /usr/bin/rustc1.98.1, commit
48a229ceaefd4985c50990b14116b6d856af0985, x86_64-unknown-linux-gnu. Both independent
passes used edition2024, JSON diagnostics, denied overflowing_literals and
arithmetic_overflow, and overflow-checks=yes. Each performed64 compile invocations:
32 BUILD link and32 genuine metadata-only CHECK. All20 valid cases produced actual
BUILD and CHECK artifacts; all20 BUILD runtimes exited0 with the exact literal
`canonical copy oracle ok\n` and empty stderr. Every invalid case exited1 in both
phases without artifacts. Primary diagnostic code multiplicities and exact
structured level/code/message values were checked, including N27's uncoded
`lifetime may not live long enough` error. Full raw stdout/stderr, arguments,
artifact presence, sizes and hashes are retained.

One explicitly authorized harness-only correction is preserved. The first P01
attempt stopped because successful bin metadata-only CHECK produced a real regular
zero-byte .rmeta, contradicting a nonempty-artifact assertion. Root inspected the
actual metadata-only argv, status0, empty diagnostics and artifact. The corrected
rule accepts genuine regular .rmeta presence even when zero bytes and records its
actual size/hash. It never substitutes metadata+link or changes crate type, source,
expected values, categories, invalid error checks or runtime checks. The entire
first attempt and STOP remain intact. First results SHA256 is
`baa88df509994ffc76ad312494fe284eb6288a2d5d1b1ff813f865ef65408366`.
metadata-correction.json SHA256 is
`01874f08d21f95da526bc119099b9cedf9afc51c88f3a7d0fa8fb46b4a6c64e7`.

Root independently reran every unchanged full source with strict diagnostic
parity, real metadata-only CHECK and all20 BUILD runtimes. Independent receipts
are in canonical-copy-independent-stock32/receipts.jsonl, SHA256
`83eb9248739eb1cf30c1dd69e8fc020d5d1a86bfe590050f233012376aff6b8c`.
Its summary.json SHA256 is
`7e4f883a92a31a2da615333852ab2fe4b488d1e3b52a68e36119bdd837d845a2`.
Root separately verified all32 original dictionary fields, source bytes and both
hash domains against the observed sibling and original disk sources. This is
stock classification and design evidence only, not native acceptance, a speed
claim, executable recipe availability, or permission to bypass later proof gates.
