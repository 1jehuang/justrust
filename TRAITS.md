# Trait declarations, obligations and sealed static dispatch

Current extension: `TRAIT-DEFAULTS.md` defines executable source-provided trait
methods, universal checking of unused defaults and override-aware static dispatch.
The first-client exclusions below describe the historical checkpoint, not removal
of later delivered source-default or borrowed-receiver support.

Status: frozen design and independently classified source contract. This
commit alone is not implementation authorization. All source owners and both
independent design reviewers reported feasibility CLEAR after complete reads.
Full Rust and existing-project compatibility remain the objective. This is one common trait,
impl and predicate foundation, not a spelling-based method or Copy shortcut.
The preceding inherent checkpoint is implementation16fadabeb/evidence89c842567.

## First executable client

Admit local nongeneric trait declarations with required associated function and
by-value receiver method signatures, local nominal trait implementations,
generic impl headers and positive `T: Trait` predicates on admitted callable
and impl type parameters, including conditional trait impl prerequisites. Existing inherent value methods remain unchanged.
Required trait items have no executable body. Every implementation body uses
the common callable enumeration and all-unused universal analysis gate.

First source grammar: `trait Name { fn item(self, ...)->...; fn make(...)->...; }`,
`impl<T: Bound + Other> Trait for Local<T> { ... }` and inline positive
`T: Trait + Other` bounds on ordinary function/impl/method parameters. No where
clauses, trait type arguments or trait-method type parameters in this client.
Admit fully qualified `<Type as Trait>::item`, bound-qualified `T::item` and
by-value dot selection with exact scope-sensitive candidate rules. Existing
inherent precedence is preserved. Concrete trait lookup uses locally declared
or explicitly imported visible traits, never every same-name registry trait.
Ambiguous candidates reject; no choose-first search. Trait impl items have the
trait visibility and cannot invent `pub` declarations. Required trait signatures
are reference-free in this client; existing ordinary nongeneric body-local
reference support is unchanged. Explicit arguments for
existing generic inherent methods keep their current boundary.

Do not claim trait generics, generic trait methods, default method bodies,
associated types/constants, supertraits, dyn/vtables, trait objects, negative
impls, specialization, auto traits, borrowed receiver adjustments or generic
references. Excluded valid Rust gets an explicit supported-boundary diagnostic.
Copy/Clone/Sized language items are separate clients of this same foundation.
No permission follows from an identifier spelled Copy, Clone or Sized.

## Identity and declaration authority

Extend the authoritative declaration registry with distinct Trait, TraitItem and
TraitImpl identities. A trait Self binder is real and owner-qualified. Impl and
method scopes retain ordered GenericEnv identity and total-arity accounting.
Trait method schemas retain their declaring trait/item and receiver metadata,
not a synthesized free-function name. Stable encoding includes semantic trait
and item paths and canonical impl self-patterns, never allocation/discovery IDs.

Proposed shared semantic terms:

- `TraitRef { declaration: DeclId, self_ty: Ty, args: Arc<[Ty]> }`.
- `Predicate::Implements(TraitRef)`.
- A privately constructed validated PredicateEnv, separate from binder-only
  GenericEnv, tied to the exact declaration and immutable registry/world.
- `Callee::Direct(CallTarget)` or
  `Callee::TraitItem(TraitCall { trait_ref, item, method_args })` in the SAME
  checked expression and CFG call operation.

Trait generic argument storage is canonical even if nonempty arguments are an
initial source boundary. Authoritative constructors check exact arity, binder
membership, trait/item ownership, reference restrictions and canonical nominal
metadata. Diagnostic names never establish identity or semantic permissions.

## Trait Self and schema wellformedness

Trait Self is not silently implicitly Sized. Required signatures use a distinct
schema-wellformedness mode in which Self may be unsized, with no BODY, Paths,
layout, native instance or ownership permission for the trait declaration item.
Top-level Self parameter/result positions do not invent a Sized supertrait.

This is not permission to form arbitrary aggregates containing possibly-unsized
Self. Array elements, tuple positions and nominal generic arguments obey their
actual Sized formation requirements, including zero-length arrays. Unsupported
schema forms are rejected explicitly rather than checked with a fake sized
witness. No general ?Sized value/layout support is implied.

At an admitted dispatch site ordinary caller type parameters retain their
existing implicit Sized contract and local nominal impl Self is sized. After
simultaneous substitution, instantiated signatures and implementation bodies
use the ordinary complete value-type validation. Required declaration schema
validation must not be confused with an executable body proof.

## Universal predicate and dispatch authority

A generic caller proves a trait goal from its exact authoritative PredicateEnv,
not from a convenient concrete instantiation or a method of the same spelling.
Caller parameters are rigid. Assumption evidence is distinct from selected
concrete impl evidence, and is tied to exact world, environment and goal.

Universal TraitItem calls retain trait/item identity, Self and ordered actual
arguments. Validate their canonical substituted declaration signature and exact
bound obligation in every live and dead typed node. Do not turn them into a
Direct target before a unique concrete implementation has been justified.

A single fallible shared semantic service proves goals and resolves dispatch.
The service distinguishes Proved, NotProven and actual errors. Invalid metadata,
ambiguity, recursion limits and resource exhaustion never become NotProven or
success through swallowed errors. Predicate normalization and signature
substitution are simultaneous and owner-qualified, returning caller actuals
verbatim instead of recapturing same-owner recursive parameters.

## Implementation conformance and coherence

Before publishing successful body facts, validate every trait and impl header,
including unused declarations. Check item ownership, missing/extra/duplicate
items, receiver mode, exact parameter/result signature after Self and impl
substitution, region/reference contracts and predicate requirements. Local
`mut self` binding is not a receiver-mode mismatch: a required `self` method
may be implemented with `mut self`, preserving the implementation local
mutability without changing its trait signature. An impl
cannot strengthen the method's required predicates or bypass visibility. Trait impl body context
also receives the validated header-derived Self:Trait assumption under its
declared prerequisites, never evidence from its own body success.

Only admitted local nominal impl heads are supported initially. Every generic
impl parameter must be constrained by the admitted header pattern. Global
coherence compares same-trait impl heads with freshened two-sided first-order
unification and occurs checks, even if item names differ. Candidate selection remains a separate one-way
match against rigid actual types. Do not treat absent implementations or
currently unprovable where clauses as evidence that overlapping heads are
disjoint. Any bound-sensitive disjointness outside the solver's represented
rules is an explicit boundary, never choose-first selection.

The immutable validated trait world is published before body analysis. Cyclic
ordinary goals are not self-justifying evidence. Active-goal recurrence fails
closed with a precise cycle/resource boundary, with no coinductive Copy or
implicit success. Exact function-instance recursion remains supported and is
not an obligation cycle.

## Concrete instance and native trust path

Exhaustive checked-IR instantiation substitutes every trait reference,
predicate, item argument, signature, nominal owner and existing typed metadata.
The same resolution helper used by instance discovery selects one validated
impl and discharges its instantiated prerequisites. Compare its resulting
signature with the substituted trait declaration before publishing Direct.

One exhaustive `close_checked_calls` pass applies to EVERY concrete checked
body, including reused nongeneric bodies, not only instantiated templates. It
discharges all Direct callee predicates and closes all TraitItem calls with
canonical signature comparison. Instance frontier discovery uses the SAME
resolution service before enqueueing targets.

Every closed call, including calls in dead metadata, must be Direct with a
canonical concrete target. ClosedCfg/ConcreteCfg reject residual TraitItem or
unproved obligations. Concrete lowering runs anew, concrete analysis succeeds
for the exact immutable CFG and native emission accepts only its existing
sealed capability. Backend does not select methods or interpret predicates.

CHECK demands all declaration and universal/concrete analyses without OBJECT.
BUILD never publishes executable output before every required proof succeeds.
Required trait signatures are not seeded as native functions or fake bodies.
Existing nongeneric functions and inherent associated bodies retain their
all-unused contract and no user compilation delegates to external tools.

## Shared facts and future canonical Copy

Keep the SAME ownership Paths, Transfer, join and worklist. Param shape stays
an opaque leaf even if a future validated predicate proves Copy. One fallible
metered facts authority binds exact registry/world, GenericEnv, PredicateEnv,
phase and shared PlanBudget. Thread this authority through frontend extraction
and repeats, checked-IR repeat validation, CFG Copy/index/repeat checks,
lowering consume/index decisions and ownership indexed moves. Direct Ty::is_copy
must not become a second authority for bound-dependent permissions.

A future Copy client must resolve the real canonical language-item identity,
prove required Clone/supertrait contracts and nominal eligibility/Drop rules,
and never infer nominal Copy only from field shapes. Explicit private Move
continues consuming even for a Copy-proven type. The current return-analysis
synthetic Copy is initialization inspection, not a Copy permission: separate
that read-only inspection before tightening operand Copy validation.

## Resource and cache contract

Use the SAME shared PlanBudget from schema construction through obligations,
instances and sealed native planning. Existing caps remain32 total visible
parameters,4096 combined applications,128 active depth,1M expanded nodes,
32M shared work and128MiB retained/scratch metadata. Add at most4096 distinct
canonical obligation/cache entries across the snapshot and256 canonical
predicates per callable environment or declaration header, counting inherited
and implied predicates. Aggregate world storage remains under shared nodes and
bytes limits.
Constructed rich diagnostic payloads share a64KiB snapshot-total allowance,
charged before construction; bounded UTF-8-safe truncation is explicit, never a
hidden uncharged rich-format intermediate. A legacy `String` error boundary may
still copy one fixed resource-exhaustion message of at most80bytes per failed
conversion when that allowance is exhausted. This constant-size transport
exception contains no source text, goal trace or formatting and never turns
failure into success or renews any semantic budget. It is not a whole-process
allocation bound. These are supported resource boundaries, not Rust language
limits.

Charge rejected candidates, pairwise coherence, structural equality, name-byte
comparisons, assumptions, cache hits and misses, recursive goals and canonical
keys before work or growth. Reserve vectors, maps, substitutions, proof nodes,
retained signatures and temporary scratch before allocation. Reused DAGs are
not free. No fresh allowance per impl, body, goal or worker and no lock across
await or recursive proof execution.

Memo keys include full canonical goal, immutable world identity/generation,
exact predicate environment and phase. Active, proved, disproved and errored
states must not alias. Never cache resource exhaustion as semantic false or
reuse evidence from another environment/snapshot. Stable persistent semantic
keys encode paths and canonical terms, not process-local world IDs.

## Shared API and ownership freeze

The new src/compiler/traits.rs owns the immutable validated world, PredicateEnv,
ValidatedFacts, typed FactsError, private proof records and ONE obligation
engine. IR owns semantic terms and exhaustive typed visitors. Frontend owns
raw authoritative headers and the proof-free schema/interner adapter. Root
owns graph and instance gates. Ownership retains its SAME solver and exact
facts capability. Backend accepts only the existing concrete native seal.

The single constructor is ValidatedTraitWorld::build(HeaderSet,
Arc<dyn SchemaCx + Send + Sync>, &PlanBudget), returning Result<Arc<Self>,
FactsError> only after complete header/conformance/coherence validation.
The world retains the raw adapter and implements TraitCx itself. There is
NO BoundTraitCx, frontend TraitAdapter, arbitrary service-binding factory or
second frontend obligation solver.

SchemaAdapter retains raw registry, raw schemas, canonical interner and the
same ledger ONLY. It MUST NOT retain PlanCtx, world, validated signatures,
facts or proof service. PlanCtx retains the world and validated signatures
OUTSIDE that raw core. Facts retain Arc<ValidatedTraitWorld>. World never
retains facts or proof records with an Arc backedge. Predicate/proof/cache
records use nonowning WorldId. This establishes an acyclic retained service.

Raw GenericSchema { env: Arc<GenericEnv>, signature: Signature,
predicates: Arc<[Predicate]> } never contains validated PredicateEnv.
The exact AST-free records are:

- HeaderSet { traits: Arc<[TraitHeader]>, items: Arc<[TraitItemHeader]>,
  impls: Arc<[ImplHeader]>, callables: Arc<[CallableHeader]> }.
- CallableHeader { declaration: DeclId, schema: GenericSchema }.
- TraitHeader { declaration: DeclId, self_param: ParamId,
  env: Arc<GenericEnv>, items: Arc<[DeclId]> }.
- TraitItemHeader { declaration: DeclId, trait_decl: DeclId,
  receiver: ReceiverMode, schema: GenericSchema }.
- ImplHeader { declaration: DeclId, trait_ref: TraitRef,
  schema_env: Arc<GenericEnv>, predicates: Arc<[Predicate]>,
  items: Arc<[(DeclId, DeclId)]> }, mapping each required trait-item DeclId
  to its implementation callable DeclId.

Frontend AssociatedSemanticOwner adds trait_decl: Option<DeclId> and
trait_item: Option<DeclId>. Inherent owners have BOTH None; trait impl methods
have BOTH Some. Validate paired presence, item owner equals trait_decl, impl
header trait identity and exact item-to-callable mapping. Stable encoding
resolves semantic trait and required-item paths, canonical self pattern,
impl-site module and receiver metadata, never raw IDs or source indices.
Required TraitItem identities are registered for schema lookup only and are
excluded from callable_count/body/native roots.

SchemaCx has shared-reference fallible methods declaration_info(id),
instantiate_schema(schema, ordered_args, caller_env, budget), and
validate_nominal(ty, env, budget). SchemaDecl exposes authoritative declaration,
kind, own_env, visible env, owner, module, public and receiver mode.
InstantiatedSchema returns signature and raw predicates. This callback performs
simultaneous structural substitution and canonical nominal formation ONLY,
never proof or a read of the world being constructed. Publish validated
GenericSignature.predicates only AFTER world construction.

SchemaCx also exposes readonly budget()->&PlanBudget and
registry_snapshot()->SnapshotId. build verifies same ledger authority and
registry identity BEFORE any callback query, so declaration_info cannot charge
an unrelated allowance. These process-local identity getters are not persistent
semantic cache keys.

Exact callback signatures (all methods take &self) are:

- budget() -> &PlanBudget.
- registry_snapshot() -> SnapshotId.
- declaration_info(id: DeclId) -> Result<SchemaDecl, FactsError>.
- instantiate_schema(schema: &GenericSchema, args: &[Ty], caller: &GenericEnv,
  budget: &PlanBudget) -> Result<InstantiatedSchema, FactsError>.
- validate_nominal(ty: &Ty, env: &GenericEnv, budget: &PlanBudget)
  -> Result<(), FactsError>.

Readonly ValidatedFacts getters are world()->&Arc<ValidatedTraitWorld>,
service()->&dyn TraitCx (derived from world), env()->&Arc<GenericEnv>,
predicates()->&Arc<PredicateEnv>, phase()->FactsPhase, budget()->&PlanBudget,
declaration()->DeclId, target()->Option<&CallTarget> (Some for Closed only),
and registry_snapshot()->SnapshotId. There are no setters or public new.
All authority comes from the private world factories. TraitCx::prove_goal takes
&Predicate and the exact &ValidatedFacts; other operations take TraitRef,
TraitCall or CallTarget as appropriate. All return typed Result, never a
successful boolean substituted for evidence.

TraitCx methods validate_trait_ref, instantiate_trait_signature, prove_goal,
discharge_direct and resolve_trait_call take exact ValidatedFacts and return
Result with typed FactsError. ProofOutcome is Proved(private exact-goal evidence)
or NotProven. DirectCallProof/ResolvedTraitCall expose readonly canonical
callee/target and signature. World verifies exact world, registry, subject,
phase and ledger before queries. Facts.must_copy remains fallible, preserving
intrinsic Copy behavior only in this client, with no trait Copy grant.

Factories on Arc<ValidatedTraitWorld> are callable_facts(decl) for Universal,
schema_facts(item) for Schema, and instance_facts(target) for Closed. Private
facts derive exact env/predicates from the subject, not caller arguments.
Closed instance facts validate actuals and PROVE ALL declaration requirements
under empty assumptions BEFORE publication, including entrypoints with no
calls in their bodies. Nongeneric roots use instance_facts with empty args.
There is no phase flag that silently erases generic parameters or predicates.
Schema cannot lower bodies, query executable Copy, build Paths or reach native.
Universal retains exact GenericEnv and assumptions. Closed retains exact target,
closed types, empty environment and empty assumptions after entry proof.

cfg::lower_template(Arc<CheckedFunction>, Arc<ValidatedFacts>, &PlanBudget) and
cfg::close(Arc<Function>, Arc<ValidatedFacts>, &PlanBudget) retain exact facts.
TemplateCfg/VerifiedTemplateCfg expose facts(), deriving env()/budget().
Existing verify_template/analyze_closed execute the actual SAME solver, never
accepting caller-provided successful analysis for a different body. ConcreteCfg
binds exact immutable CFG, successful analysis and exact closed facts.

ir::close_checked_calls(source, facts, substitution_cx, budget) is the ONE
exhaustive all-body closing visitor. instantiate_checked invokes it after
substitution, and nongeneric reuse invokes it before fresh concrete lowering.
Instance discovery shares world resolution, not a second selector/solver.
Required trait schemas never enter callable_count or fake body/native slots.

## Frozen stock acceptance corpus

The authoritative corrected artifact is traits-oracle-corrected/corpus.json,
SHA256 bd6a43c218ef8f2b6596ee2281ddd28fa30d948448794fe5b3ea7904d0d5c6ef.
It contains EXACTLY32 cases:16 supported candidates,12 invalid Rust cases and
4 legal exclusions. All32 were independently rerun by the coordinator against
stock BUILD and metadata CHECK, with source SHA/bytes, output artifacts and
exact error-code multiplicities checked. The20 stock-valid programs actually
executed with exit0, exact `traits oracle ok\n` stdout and empty stderr.
The12 invalid cases reject in BOTH phases with exit1 and no output artifacts;
the combined missing-bound case observes E0277 twice and E0599 in both phases.

One source was corrected BEFORE this freeze and before ANY native execution:
the proposed default-body exclusion B29 originally used by-value trait Self
in a body without a Sized bound and stock correctly rejected it with E0277.
The authorized replacement is a STATIC default function, which stock accepts
and this client explicitly excludes. Original initial corpus/results remain
preserved in traits-oracle; correction.json records original source, observed
status and authorization. ALL31 other complete dictionaries are byte-identical.
This is an observed stock-classification correction, not a native-failure fix.
Corrected raw results SHA256 is
23620a7e3e528b18e8232f12773993e20d425ee60bbbdb228165bbaacfec8167.

No later source growth, correction or expected-outcome reclassification may
hide native failures. Project harnesses must embed these exact32 full sources,
classifications and stdout strings. The bounded corpus is not full Rust proof.

## Acceptance before implementation authorization

Owners must agree exact shared API and source grammar, then read this complete
contract and report concrete feasibility. Independently stock-classify a frozen
bounded corpus before native implementation, separating valid support, invalid
Rust, legal exclusions and resource boundaries. No source growth or correction
to hide native failures. Include unused invalid bodies/headers, generic
forwarding, rigid caller assumptions, nominal identity, dispatch, conformance,
coherence, privacy and wrong-bound cases.

Require production+fixture compile-ready checkpoints before focused grouped
regressions, integrated hosted check/test/strict all-target lint/fmt, actual
native examples, static soundness/resource reviews and an independently parsed
cumulative native campaign. Export selected redacted profiles, preserve all
historical findings, commit/push working stages and raw evidence separately.
No experimental install or performance/default claim without comprehensive
pinned before/after measurement. Full Rust remains open throughout.

## Implemented bounded client and retained acceptance

The required-item client now uses the frozen canonical world, conditional
obligations, exact subject facts and common all-body call closure. Required
trait items remain signature-only, while implementation bodies use the same
universal ownership analysis and closed native instance seals as free and
inherent functions. Custom traits named Copy do not grant copy permission.

The exact corrected32 sources and four graph regressions pass, including
worker-deterministic conditional dispatch and the absence of native slots for
required signatures. All1368 historical and new hosted tests pass with check,
strict all-target lint and formatting clean. The real traits example executes
with8 workers and exact expected stdout. Independent cumulative native receipts
remain a separate gate, not a conclusion from these focused tests.

One unchanged historical associated-items trait_dispatch source was promoted
only after stock plus native workers1/8 BUILD, CHECK and runtime parity. Its
full-source SHA256 is
2b892fda952e7215f317df893b69d7fc2f079c3bcf8b2c393e8d4fce5d66d71b.
The historical corpus is now19 supported,10 invalid and3 legal exclusions.
The original frozen32 trait corpus and its classifications are unchanged.
Copy/Clone language-item semantics, borrowed trait receivers, generic traits
and trait methods, defaults, associated types, standard libraries and existing
project acceptance remain open. This is not full Rust or a speedup claim.
