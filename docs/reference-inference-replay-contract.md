# Original-call inference replay and original-schema accessor

2026-10-17. **Bounded contract candidate for whole-file review. Not implementation or admission GO.**
This document specifies the next source-inference authority. Contract review is static. The separately preserved stock/native baseline observations are not new native acceptance, and this document changes no production source or supported syntax.

## 1. Evidence and actual loss points

Prerequisites are `docs/reference-inference-source-order-20261016.md` and `compiler/GENERIC-CARRIERS.md`. The six-case evidence committed in b929df0c3 and the separately frozen shared-local baseline committed in 8f42de61b supply the observed requirements. The source anchors below identify inspected implementation, not completed replay implementation.

* `frontend.rs:7025-7350`: InferKind/Inference are frontend-private, coupled to Checker. Slots, constrain/merge/resolve exist; finish fences newly inferred root Ref actuals.
* `7649-7677`: infer_call_args applies expected result first, scans all arguments, then finishes. No pre-checking frontier survives.
* `7823-7846`: Expectation distinguishes None, HasType and CastableToInt. Only HasType is hard; plain Ty/Option carries no contextual origin.
* `9885-10119`: call_argument can insert mutable reborrows. finish_checked_call resolves trait selection, fills region rows, derives all expectations from the supplied signature, checks arguments, patches pending reborrows to TwoPhase, and emits final traces.
* `10433-10516`: expr_without_cast can independently insert mutable or mutable-to-shared reborrows from a hard Ref expectation. Source borrows start Immediate. Both paths must consume the same original-stage decision.
* `11213-11297`: direct-call checking consumes explicit applications or inference, constructs final target/signature and discharges requirements before finish_checked_call.
* `ir.rs:674-888`: CallEvidence retains final argument IDs, expectations and region rows. Its validator compares expectations to canonical completed parameter heads. The final completeness scan requires evidence for root-Ref actual calls, not all original source occurrences.
* `ir.rs:1132-1278`: BodySubstitutionProof retains exact template/target/generated shape and validates Closed world/ledger/target correspondence. Keep this transport, not a new lifecycle.
* `traits.rs:222-306,342-410,3567-3688`, `ir.rs:283-320`: SchemaCx has declaration_schema, but TraitCx has no original-schema read. World callable_schema privately derives original env/signature plus validated requirements. ValidatedFacts supplies exact world/snapshot/subject/environment/phase/ledger.
* Directly inspected `frontend.rs:3750-3830,4750-4784,9515-9573,11470-11545`: PlanCtx retains program:Arc<Program>. callable_body indexes a supplied Program by registry source indices without exact membership validation. Publication currently retains no original AST/context receipt. Unannotated Let passes None before introducing the binding; annotated Let converts its annotation first.

Final-signature equality is necessary but cannot recover original omittedness, result context or source-order adjustment. An enum waiver, bool, adjusted-IR self-consistency check or retained arbitrary hint is insufficient.

## 2. Single authority path and explicit trust boundary

Use the existing plan-owned Program, canonical registry CallableSource and private source-publication boundary. No global AST-ID rewrite or second full AST clone is required. An original occurrence is `(private source owner, caller declaration, structural child path)`, not an AST address, spelling, hash or checked ExprId. Include Apply-to-Call and ordered argument roles. Keep original source and proof retention acyclic.

```rust
// Proposed compiler-private records. All constructors private.
struct OriginalBody {
    identity: OriginalBodyId,
    origin: OriginalBodyOrigin,
    caller: OriginalCallerBinding,
    bindings: Arc<[CheckedBindingOrigin]>,
    correspondence: Arc<[SourceCheckedOccurrence]>,
}
enum OriginalBodyOrigin {
    Source { program: Arc<Program>, source: CallableSource },
    Recipe { catalog_body: CatalogBodyHandle },
}
struct SourceNode { owner: OriginalBodyId, path: Arc<[SourceChild]> }
struct SourceBinding { declaration: SourceNode }
enum SourceChild {
    Statement(u32), Tail, Initializer, Argument(u32), ApplyValue,
    ReturnValue, Block, Then, Else, Arm(u32), Operand(u32),
}
```

OriginalCallerBinding binds exact world, registry, caller declaration/instance, original binder, authoritative predicate environment including sentinel role, FactsPhase and same PlanBudget. CheckedBindingOrigin binds the source declaration/parameter occurrence and lexical scope to checked LocalId, parameter index or source annotation, initializer/statement correspondence and established type. SourceCheckedOccurrence binds the source expression/role to actual checked nodes and their adjustment relationship. These are source-publication facts, not raw user proof fields.

`OriginalBodyId` is an opaque retained identity token with private construction and pointer-identity comparison while its Arc remains alive, not a persisted bare address, reusable integer or value-equal empty record. SourceNode retains that token, not the enclosing OriginalBody, so owner/binding paths form no self-cycle. Charge its allocation/header and retained references on the same ledger.

Proposed Source constructor: `PlanCtx::publish_original_body(declaration, facts, checked_payload, correspondence, budget)`. It takes **no arbitrary Program/body**. It checks plan world/snapshot/facts/caller/binder/ledger and derives the body through self.program and the validated Free/Associated registry mapping. The checker producing checked_payload must have selected that same owned body. Merely calling this constructor after checking another supplied Program is forbidden.

**Settled first-CLI source entry:** one private canonical checking entry always selects plan.program through its canonical registry mapping and passes a private canonical-source capability into Checker. All root-graph paths admitting the new domain must use this entry. Existing public Program-taking adapters preserve their old domain and return Unsupported for new inferred-reference applications before publication. They do not ignore conflicting input, compare input pointers, or require whole-Program equality. Normal cloned-Program legacy callers remain supported. No public adapter can mint the canonical-source capability.

The trusted publication boundary associates successful ordinary checking with the exact canonical input, lexical bindings and completed payload. Replay independently checks inference/context/adjustment transitions against those immutable facts. Private fields alone do not prove this association. Audit membership and correspondence construction before GO. Recipe authority must similarly derive from the canonical catalog-owned body, not arbitrary generated IR; its constructor is unresolved. OriginalBody retains no CheckedFunction, CFG, template or capsule, avoiding a source/proof backedge. It may retain the existing Program Arc or a bounded projection certified from that exact owner.

**Settled immutable publication:** private `CanonicalSourcePublication` retains the exact **final** `Arc<CheckedFunction>`, source-only `Arc<OriginalBody>`, source-to-checked mapping and validated replay. Construct it only after the existing canonical metadata transformation has completed and correspondence plus replay succeed on that final Arc. No later canonicalization may silently replace its payload. The private canonical pipeline may use the existing common unsealed metadata transform internally, but must run mandatory structural/canonical checks and original/new-domain evidence validation on the final Arc before publication. Public raw canonicalization/validation wrappers stay strict; no public skip flag or alternate unvalidated visitor is introduced. It is the canonical-source entry's immutable publication association inside the existing evidence lifecycle, not a second body/ownership proof. No one-shot source receipt or payload snapshot is needed for original identity. Consumers check `Arc::ptr_eq` against the retained final source Arc plus exact owner/facts/world/environment/phase/ledger and validated payload association. `Arc::make_mut` edits produce a separate Arc while publication retains the original, so an edited clone or transplanted owner cannot reuse publication authority. New-domain raw public bind_call_evidence without this canonical publication cannot mint original authority.

The retention DAG is explicit: publication -> final CheckedFunction Arc, source-only OriginalBody, mapping and validated replay; existing capsule -> exact source Arc, source-only owner, validated replay and facts. CheckedFunction must retain neither publication nor capsules. OriginalBody retains no produced source, publication, capsule, CFG or template. A capsule never retains its enclosing publication or another enclosing capsule. Existing VerifiedTemplateCfg/BodySubstitutionProof transport retains the original evidence through its current sidecars. Existing generated-shape snapshots remain for substitution validation only, not a new original-source identity snapshot. All new retained associations charge the same ledger.

## 3. Original context and preadjustment authority

```rust
enum ResultInput {
    Absent { site: SourceNode, route: ContextRoute },
    Hard { site: SourceNode, term: SourceTypeTerm, route: ContextRoute },
    SoftInteger { site: SourceNode, integer: IntTy, route: ContextRoute },
}
struct ContextRoute { edges: Arc<[ContextEdge]> }
enum ContextEdge {
    AnnotatedInitializer { binding: SourceBinding, annotation: SourceNode },
    UnannotatedInitializer { binding: SourceBinding },
    BodyResult { owner: DeclId }, ReturnOperand { statement: SourceNode },
    BlockTail { block: SourceNode },
    OuterCallArgument { call: SourceNode, argument: u32 },
}
```

These are untrusted candidate descriptions until checked against the retained owner/path. Rederive context from actual parent roles and root authority. An annotation term comes from retained source syntax converted under exact caller binders/regions. Body/return context comes from the original caller schema. Outer-argument context comes only from the exact enclosing call's replayed *before-argument* frontier, never its final signature. Validate dependency order, not checked-arena numeric order. No skipped/resetting parent edge, sibling annotation, eventual branch join or final let type may supply authority.

None requires proof too. Initial domain: call is exactly the initializer of original Stmt::Let with `ty == Type::Infer`, so 9534-9539 establishes None before the new binding is introduced. The final local type at 9541-9549 is not evidence of that None. Argument-local annotations refer to separate earlier bindings. Unknown parents must not default to None. Hard Unit/Never is still a hard context. Castable integer hints are not hard reference expectations. General assignment, aggregate, branch/match, nested call, cast, loop and associated-call context transfer needs explicit rules; existing associated dispatch does not uniformly propagate expected context. Legitimate unimplemented routes stay Unsupported, not invented None.

For a source Var, derive preadjustment type from its exact lexical binding and checked origin. Parameter origin roots in canonical caller schema. Annotated local origin roots in original annotation plus ordinary initializer validation and source-to-local correspondence. Inferred local origins require exact earlier checked initializer/assignment authority, initially Unsupported. A copied arbitrary `locals[id].ty`, InferTerm or adjusted Borrow node is not independent evidence. Source Borrow/Deref needs source place/operand and validated referent provenance, not reconstruction from inserted nodes. Replay never evaluates expressions or generates a second runtime read.

## 4. Concrete raw replay and inference-state shapes

```rust
struct OriginalCallReplay {
    source_call: SourceNode, checked_call: ExprId,
    application: OriginalApplication, result_input: ResultInput,
    initial: FrontierId, arguments: Arc<[ArgumentReplay]>, finish: FinishReplay,
}
enum OriginalApplication {
    Direct { declaration: DeclId, actuals: ActualForm },
    TraitItem { original_item: DeclId, receiver: Option<SourceNode>,
        self_term: SourceTypeTerm, trait_actuals: Arc<[SourceTypeTerm]>,
        method_actuals: ActualForm, selected: Callee },
}
enum ActualForm {
    Omitted { site: SourceNode },
    Explicit { site: SourceNode, terms: Arc<[SourceTypeTerm]>,
        regions: Arc<[TypeActualRegion]> },
}
struct ArgumentReplay {
    argument: u32, source: SourceNode, preadjustment: OriginalValueEvidence,
    before: FrontierId, expected: ReplayTermId, adjustment: AdjustmentReplay,
    checked_value: ExprId, after: FrontierId,
}
struct OriginalValueEvidence {
    source: SourceNode, derivation: ValueDerivation, term: ReplayTermId,
}
enum ValueDerivation {
    Binding(SourceBinding), SourceBorrow { kind: BorrowKind, place: SourcePlaceId },
    SourceDeref { operand: SourceNode }, SourceLiteral { literal: SourceNode },
}
enum AdjustmentReplay {
    Identity { value: ExprId },
    Reborrow { value: ExprId, carrier: Option<ExprId>, place: Place,
        kind: BorrowKind, timing: BorrowTiming },
}
struct FinishReplay {
    frontier: FrontierId, actuals: Arc<[Ty]>,
    defaulted: Arc<[DefaultingStep]>, target: Callee,
}
struct InferVar { call: SourceNode, binder: BinderId, index: u16 }
enum ReplayTerm {
    Var(InferVar), Rigid(Ty), Integer, Unknown(UnknownDomain),
    Ref { kind: BorrowKind, referent: ReplayTermId },
    Tuple(Arc<[ReplayTermId]>), Array { element: ReplayTermId, len: usize },
    Nominal { declaration: DeclId, actuals: Arc<[ReplayTermId]> },
}
struct Frontier { call: SourceNode, slots: Arc<[SlotBinding]> }
struct SlotBinding { variable: InferVar, value: Option<ReplayTermId> }
```

All new names are proposed internal records, not existing implemented APIs. SourceTypeTerm retains original type/lifetime syntax occurrence, owner-qualified meaning and region occurrences. Partial explicit `_`/holes remain Unsupported unless separately specified; never conflate them with Omitted. Retain even zero-arity source form where grammar permits it. Fresh callee-use variables are keyed by call and original binder slot. Caller Params remain rigid and owner-qualified, including recursive calls and equal eventual concrete types. Identity does not assert Copy/Move, which remains ordinary lowering/ownership behavior. Explicit source Borrow is not an inserted reborrow. No RawTy/catch-all derivation grants permission.

Logical frontiers include initial, every before/after and finish. Immutable deltas/checkpoints may replace full vectors only with complete validation/materialization accounting. Validate all nodes, unused terms/frontiers, edges and paths, with no cycles or later-argument evidence in an earlier frontier. Final actuals are replay outputs, not seeds.

## 5. Authoritative original-schema read

```rust
impl ValidatedFacts {
    fn original_callable_schema(self: &Arc<Self>, subject: OriginalCallableSubject<'_>)
        -> Result<Arc<OriginalCallableView>, FactsError>;
}
enum OriginalCallableSubject<'a> { Direct(DeclId), TraitItem(&'a TraitCall) }
struct OriginalCallableView {
    facts: Arc<ValidatedFacts>, subject: OwnedOriginalCallableSubject,
    declaration: DeclId, schema: GenericSchema,
    source_regions: Arc<SourceRegionEnv>, receiver: ReceiverMode,
    binding: OriginalSchemaBinding, // private, derived only by the world
}
```

Readonly accessors: declaration, generic_env, signature, predicates, source_region_env, receiver, subject. No raw SchemaCx, mutable schema, public constructor, arbitrary validation callback or caller success flag. This is header-read authority, **not** executable instantiation, refactual admission or body-family proof.

World implementation must run check_facts with executable requirement, reject Schema phase, validate exact world/registry/ledger and registered predicate environment or its genuine world-owned empty-context sentinel. Bind full caller FactsSubject, including ordered ClosedInstance actuals, not an asserted env index. Validate subject kind/DeclId and original binder/scope order. Direct obtains original schema via existing private callable_schema and validated requirements, with world-build declaration_schema agreement checked, never via the final instantiated signature. Obtain original source-region schema from validated declaration carriers. The view binds exact world, snapshot, original declaration/header, binder, caller subject/env/phase and same ledger. Keep sentinel handling explicit, not an ordinary indexed environment. Charge lookup/validation/cloning/retention and revalidate authority on cache hits. No schema read triggers body queries, instance admission or world-to-body callbacks.

First implementation has no context-view cache: each metered transient OriginalCallableView retains caller facts, which retain the world, so strongly caching that view inside the same world would form a cycle. World retains only its pure original header tables. Any future context cache must be outside its retained world or use audited weak ownership with authority revalidation; cache hits still charge work.

First new replay domain is Direct only. Existing trait behavior retains its current proof path without new inferred-reference permission. Future TraitItem replay must retain original abstract item/Self/trait/method/receiver roles before current finish_checked_call resolves selection, independently validate selection in original facts and map provider binders/ordinary actuals. Provider identity alone or Self disguised as an ordinary omitted actual is insufficient. The enum shape above reserves required roles, not completed trait semantics.

## 6. One kernel and deterministic replay before existing sealing

**Settled native graph guard and provider authority:** the canonical native graph requires a private CanonicalSourcePublication association for **every Source body**, including bodies with only legacy calls. Provider classification comes from the authoritative world's callable_provider, never a caller flag. A raw typed-IR capsule is not a substitute for this association, even if its mutable trace is rewritten NoFirmReference -> FirmReference to satisfy the old completed-signature validator. Canonical Source publication validates its original-source classifier before any native graph consumer accepts the body. Public Program-taking adapters retain old-domain typed-IR behavior but cannot publish that native Source association or grant new inference permission. Typed Recipe/raw fixtures keep their existing typed-IR authority, existing body/Plan proofs and no new inferred-reference permission. No new Recipe source constructor is required for this first slice.

**Settled original-source classifier:** canonical publication walks every relevant original source call, including dead calls, and its actual Apply/Call association. Explicit original terms must agree with the canonical original target and use legacy validation. Omitted actuals whose original solved terms are reference-free or rigid caller Params also use legacy validation. **Any newly structural Ref, including a nested one, requires the exact narrow replay or Unsupported**, never legacy by absent record, producer enum or completed-signature FirmReference trace. Missing/mismatched publication, correspondence or classifier metadata is Invalid. Original Param forwarding remains original legacy/template authority even when closed substitution later produces Ref. Original Trait/provider Self selection remains its existing canonical-role proof, not ordinary omission; newly inferred method-actual refs remain fenced. Canonical target/schema and original source authority determine these branches. Raw typed-IR fixtures cannot supply original-source inference permission.

**Exact narrow transitions to freeze:** obtain canonical Direct schema with one own slot U and precisely parameter U/result U, no extra binder-role mapping. From original syntax prove Omitted and direct unannotated-let None. Start S0[U]=unbound, apply no result constraint. The original Var resolves to a canonical shared Ref(R), where R has neither references nor Params, via parameter or annotated-local origin. At `expectation_before_argument(0)`, expected is Var(U), so NoFirmReference; select Identity, requiring the checked operand be the same LocalId, with no inserted Borrow/Deref/carrier. Commit raw evidence Ref(Shared,R), obtaining S1[U]=Ref(Shared,R). Finish performs no numeric default and returns exactly that one actual. Check canonical final signature/target and existing region/carrier/body rules. Only this equality/one-slot transition is newly admitted initially. Mutable coercion, partial referent inference, multiple arguments/scopes, explicit inference holes and result-context solving remain out of the new grammar, not unresolved rules exercised by this slice. The shared transition API must still retain original context and before/after frontiers for later extension.

For narrow publication correspondence, derive source binding resolution using the pre-initializer lexical scope and exact shadowing stack. Parameter local must equal checked.params at its canonical schema position. An annotated local must map to the exact earlier source Let and checked Let/Declare statement, with canonical annotation type and its successful ordinary initializer check, not a same-typed sibling. The new call must be that source unannotated Let's entire initializer and the corresponding checked Let.value must equal checked_call. Match target declaration, one original argument path and its LocalId to the checked Call.args[0]; require the original Var resolves to that binding, Call ExprId names that node, and no synthetic adjustment exists. Reject duplicate source-call mapping, duplicate checked-call mapping or altered scope/order. Retain those concrete statement/local/expression links in the immutable CanonicalSourcePublication after replay/correspondence validation of the final source Arc, including dead occurrences. Consumers require that retained Arc and its validated association, not a reconstructed payload snapshot. Whole-body source/IR call classification prevents deleting the association to force legacy fallback. This defines the first-stage checks; general expression expansion and Recipe correspondence remain later gates.

Minimal split: extract term arena, slot state, merge/constrain/occurs/partial-resolve/default/finish from Checker-coupled Inference into private CallInferenceState with authoritative schema/binder and same ledger. Keep source interpretation/lexical lookup in private OriginalEvidenceCx bound to OriginalBody and checked binding origins. State owns no Checker, AST-address cache or IR builder. Producer and validator invoke the **same** `seed_explicit`, `constrain_result`, `expectation_before_argument`, `commit_argument`, `finish` transitions. Retained memo keys use owner/path, not transient address integers. No copied public solver.

Proposed `replay_original_calls(source:Arc<CheckedFunction>, original:&Arc<OriginalBody>, facts:&Arc<ValidatedFacts>, budget:&PlanBudget) -> Result<ReplayedOriginalCalls,ReplayError>` returns transient private output retaining that exact source Arc, original owner and facts binding, consumed only by existing sealing. Reuse requires Arc identity plus payload association, not address identity of a temporary &CheckedFunction. It is not a new published proof lifecycle. Before constructing any ValidatedCallEvidence:

The following is the common replay protocol for new-domain records; the first implementation exercises only the exact narrow transitions above. Whole-source coverage/classification is still mandatory, while legacy calls keep their existing validator.

1. Validate source owner/publication association, caller facts/binders/ledger and correspondence. Enumerate every original/checked call including dead syntax and metadata. Require unique coverage, no missing/orphan/foreign record; generated calls need canonical generation origin. Include Apply wrapper association.
2. Load original schema. Derive actual form/terms from original syntax. Seed explicit slots or fresh omitted variables. Rederive result input, including authoritative None; apply genuine hard result constraint before argument 0 and compare initial frontier. Never derive result input from completed Call.ty.
3. For argument i, compute partially substituted original formal under current state **before** checking it, no numeric fallback or later evidence. Compare before/expected. A known Ref head from schema, explicit actuals, genuine result context or earlier arguments can be firm; unresolved head cannot. Derive CallExpectation from this term, not final signature.
4. Rederive source preadjustment evidence. Select/check adjustment under that expectation and compare completed Local/Borrow/Deref/place/kind/referent/argument identity. An opaque T mutable Local stays an identity value use; its own evidence may solve T only afterwards, never retroactively authorize its reborrow. Check shared and mutable adjustment paths separately, not by analogy.
5. Commit argument constraints, compare after frontier, then proceed to i+1. The shared kernel must explicitly distinguish raw-evidence inference from constraints on the adjusted value: today's equal-kind Ref constrain cannot blindly compare firm &U with raw &mut U when a genuine shared reborrow is selected. This coercion-aware transition rule remains a review seam, not an invented general Rust solver.
6. Finish/default at the defined post-argument point only. Check occurs/numeric domains/owner identities, ordered actuals and final canonical target/signature using ordinary discharge. Defaulting cannot appear in prior frontiers. Replay does not bypass validate_argument, instance facts, carrier or body gates.
7. Validate explicit and inferred type-actual region rows against every exact typed carrier location, original syntax and caller Named binder. Inferred elision is fresh occurrence-derived evidence, not invented Named authority. Callee-use named variables are fresh even for recursion; caller named variables remain rigid. Existing region/carrier formation/invariance/escape proofs remain mandatory.
8. Validate BorrowTiming, activation equals this Call, argument/reason/carrier linkage, completion and once-only evaluation. Auxiliary timing carrier Local nodes cannot cause duplicate evaluation/Move. Compare actual checked expression structure, not timing self-reports. Full lowering correspondence still needs audit.
9. Validate all dead/unreferenced metadata. Compare legacy traces/region rows to replay-derived decisions and seal only after all succeed. Repeated binding replays/charges again or uses an exact private original-source seal, never a loose cache-success flag.

Producer computes ArgumentDecision before IR checking, builds nodes from it and commits afterwards. Validator recomputes and checks those nodes before the same commit. Current batch infer_call_args followed by solved-signature finish checking cannot produce this record soundly after removing the fence.

## 7. Six preserved distinctions, unchanged classification

| Witness | Required original-stage replay when its shape is eventually admitted |
|---|---|
| mutable_explicit_reborrow | Explicit T=&mut i32 is firm before arg0; preserve existing reborrow/timing and subsequent parent use. |
| mutable_first_move | Omitted T, no hard result, arg0 opaque; identity use moves mutable carrier before solving T. Later original use is ownership Invalid/E0382-class. |
| mutable_first_after_pair | First pair argument moves, then establishes T=&mut i32 for later checking; subsequent a use invalid. |
| mutable_second_reborrow | Same source-order state makes arg1 firm; b reborrow and later b use legal. |
| shared_identity | Source &value evidence solves shared T after arg0; legal with genuine source-place/origin proof. Not a shared-local Var. |
| result-expectation witness | Original `let returned: &mut i32 = id(reference)` annotation seeds T before arg0; first reborrow and later original parent use are legal subject to ordinary liveness. |

The preserved note reports explicit native success and five inferred native Unsupported cases, including two stock E0382 cases and the legal sixth result-context witness. Unsupported is not semantic agreement or rejection. Exact sources/process hashes remain in the two referenced JSONL receipts. Later acceptance must bind their exact hashes/phases/runtime/no-artifact observations to replay decisions, not substitute sources. No receipt extraction or execution was performed here. Frozen32, including X29, remains unchanged.

## 8. Existing universal-to-closed transport

Original generic replay for new-domain calls precedes universal template sealing. Every native Source body requires its canonical publication association even when all calls classify legacy; legacy call evidence retains its existing validator only within that association. All generic Source and all available Recipe bodies still undergo their existing body proofs, including unused bodies. Recipe provider identity is world-authoritative and its typed path grants no new inferred-reference permission. Nongeneric Source uses original Closed-facts canonical publication. Retain original owner and validated replay through existing private ValidatedCallEvidence, outside the source it strongly retains. CheckedFunction may retain source-only owner/raw candidates, never its capsule.

VerifiedTemplateCfg carries the original capsule unchanged. instantiate_checked and its exhaustive typed walker retarget all executable/dead metadata and validate generated shape with exact source/ExprId correspondence. BodySubstitutionProof authorizes the exact template/target/ordered-substitution/world/snapshot/ledger bridge, not literal equality of Universal and Closed env/phase. bind_call_evidence continues reusing exact template capsules after this validation. Extend association/generated-shape checks for new replay metadata; do not rerun source adjustment using closed Ty. CFG binding must match actual call block/arguments. Source calls absent from lowering still require original coverage. Closed Copy/Move selection, SAME closed Plan and ConcreteCfg/native barriers remain mandatory. No header proof supplies executable body capability.

Canonical source publication belongs to existing source typing key110 and reads only validated headers, its canonical input and ordinary checking/replay facts. It must not wait for callee universal body proofs, effect-table key115 or closed/native key114. Existing required universal-body keys111/112, effect publication115 and final all-body/native114 barriers still establish executable permission later. No new world-to-body dependency or source-publication-to-closed-analysis cycle is allowed.

## 9. Same-ledger storage, work, old/new peaks and errors

No new budget or cap increase. Shared sizing kernels serve producer, replay preflight and actual allocation paths. Count retained canonical Program lifetime without falsely deep-cloning Arc payload; count newly allocated owner/binding/correspondence/path/context/type/region/term/frontier/adjustment data, schema clones, memo tables, completeness indices, transient validation state, capsule vectors and Arc headers. Existing origin/membership/region/depth/combined-state limits remain, including 4096 origins, 1,000,000 memberships, depth128 and 128MiB analysis state. The 64 normalized-region limit is **per canonical contract**, not a caller-plus-callee combined occurrence-graph cap. Dynamic occurrence graphs remain metered by existing nodes/work/128MiB bounds. This preserves the settled GENERIC-CARRIERS interpretation without raising any cap.

For N args/K slots, full frontiers cost checked (N+1)*K plus any separately retained finish/initial copies. Delta encoding must count checkpoints/materialization/reconstruction work, not claim free linear replay. Charge every visited term/path/edge, occurs/equality operation, duplicate scan, repeated schema/region walk, cache lookup and dead row. Bound shared DAG traversal/memos and check depth on cache hits. Actual quadratic scans consume quadratic work.

Before growth/copy/traversal: checked counts, same-authority charge/reservation, fallible allocation, then operation. Include old capacity + replacement capacity + conversion scratch simultaneously, especially Vec-to-Arc and raw-to-sealed conversion. Keep old reservation live until conversion ends and original retained allocation accounted while any proof retains it. Existing ledger lifetime/reservation mechanics require audit before claiming exact peaks.

Foreign/inconsistent evidence is Invalid. Legitimate unimplemented grammar/precision is Unsupported. Exhaustion/depth/overflow/allocation is Resource, including during diagnostics. Preserve typed category before legacy String rendering with bounded/static diagnostic fallback. No exhausted charge becomes Unsupported, NotProven, semantic false, skipped descriptor or batch-inference fallback. No partial seal on failure.

## 10. Negative replay obligations and first gate

Future checks, not executed here: flip omitted/explicit while final types stay equal; forge/drop Hard or None context/annotation/parent edge; use sibling annotation; turn SoftInteger into Hard; use adjusted Borrow as source evidence; change binding under shadowing; seed arg0 from itself/later/final state; swap pair frontiers/order; duplicate/skip transitions; substitute same-named foreign binders or recursive occurrence; use foreign world/snapshot/ledger/facts/env/phase/fake sentinel; substitute provider-only trait evidence/Self role; change Local/place/Ref kind/referent/carrier/timing/activation/index; duplicate runtime reads; omit dead calls or add orphan terms; forge Named/elided/path rows; transplant equal-shaped template capsules or mutate closed adjustments; overflow/cycle/depth/memo/old-new-peak/diagnostic exhaustion. Each forged record must fail independently of final monomorphic equality. Resource stays Resource. Identical valid inputs replay deterministically regardless of workers/hash order, with fallback only at finish.

**First proposed admission only:** unary ordinary Direct original `(T)->T`, one omitted own actual, argument exactly Expr::Var known shared Ref to reference-free, Param-free referent; binding is canonical caller parameter or earlier explicitly annotated local with ordinary initializer proof. Call is exactly an unannotated direct Let initializer with derived None. No nested, branch, aggregate, loop, cast, receiver, trait/associated, inferred-local or adjustment-dependent evidence. Require all existing universal carrier/body/region and closed analysis gates. This does not promote SourceBorrow `id(&value)`, frozen X29, mutable inference, pair inference or expected-result inference. Their distinctions constrain the shared architecture but may remain honest Unsupported. Once any shape is admitted, its real move failure must reach ownership Invalid, never hide behind Unsupported.

**Resolved first-slice contract decisions:** private canonical source entry, all-Source native publication guard, public adapter exclusion from new inference authority, exact final-Arc immutable publication/DAG, authoritative provider classification, original-source legacy/new classifier and unary shared-Var transition are settled above. They remain implementation and audit obligations, not unresolved design alternatives. Before any later implementation GO, independently verify entry/root-graph plumbing, final canonicalization order, source/call/binding correspondence, exact-Arc consumer checks, original-header/env/sentinel access and same-ledger typed Resource/peak handling against this contract. Failure to establish those checks blocks admission.

**Deferred broad features, not prerequisites to implement for this slice:** result-context propagation beyond the recorded direct unannotated-Let None route; mutable/coercion-aware transitions; multiargument/multibinder inference; inferred-local/place/SourceBorrow evidence; Recipe original-source replay; new inferred trait/method actuals and abstract/provider mapping. Existing legacy proofs remain unchanged. Shared records retain their necessary roles so later extensions cannot derive source authority from closed equality. Full lowering once-only/timing audit is required for any later new adjustment grammar; the first slice requires Identity Local only.

Later authorized promotion requires independent review, exact six-receipt acceptance mapping, metadata-negative/resource checks and native workers1/8 BUILD/CHECK/runtime against preserved stock observations, without source repairs or frozen classification changes. This draft makes no compiler compatibility or speedup claim and authorizes none of those executions now.

## 11. First-slice acceptance and measured baseline

`experiments/results/20261016-reference-inference-shared-local-baseline.jsonl` preserves two separately frozen sources before admission. Both stock BUILD and genuine metadata-only CHECK passed; both stock executables exited0 with exact `inference local ok\n` and empty stderr. Current native workers1/8 BUILD/CHECK are Unsupported with no executable. Receipt SHA256 is `45712d1cd527de6164b097638aaecb15225927fe0ec46e0cf1d74fbecbd49c2f`.

| First-slice source | Exact source SHA256 | Required observation after separately authorized implementation |
|---|---|---|
| shared_parameter | `d7189ebe4f61c86f792d2af807a84162d6b34bcb4d08e318ab42d888165786b3` | Unchanged source, native workers1/8 CHECK0, BUILD0 with real ELF, runtime0 and exact literal stdout/empty stderr |
| shared_annotated_local | `357c130b2028b9f16f4e0eb48abf821187fc5fadaee1d73970e35c49c9f1df23` | Same checks, with original annotation/binding identity rather than final-local-type authority |

Use the real public file-based native compiler, not only internal fixtures. Preserve every original source, expected output and hash. Preserve existing six-case classifications outside the first domain, including explicit mutable success and legal-but-Unsupported pair/result/SourceBorrow cases. Frozen32 X29 remains Unsupported, with the entire frozen fixture and harness hashes unchanged. A later separate negative-source campaign must classify eligible-call lifetime escapes with stock and demonstrate ordinary semantic rejection, not hide an admitted invalid shape as Unsupported.

Metadata tests must cover each authority mutation in section10, especially shadowed bindings, forged absent context, removed publication, a rewritten FirmReference raw trace, owner/world/ledger swaps, edited source Arc, closed-template transplant and dead-call omission. Test public Program-taking compatibility adapters still reject the new domain without its private capability while preserving old explicit/ref-free/rigid behavior. Exercise old4096 erased-formal compatibility, source/Recipe gates, CHECK-no-artifact/BUILD-real-artifact behavior, graph key110/111/112/115/114 ordering and no OBJECT/IMAGE on semantic/Unsupported/Resource failure.

Run formatting, hosted all-target check, complete historical/integration tests and strict warning-free lint. Rebuild the actual candidate native compiler and rerun unchanged public sources at workers1/8, including real runtime/artifact checks. Add relevant resource/exhaustion and independent native corpus verification. No speedup or Full Rust claim follows from this bounded feature; performance claims still require the repository benchmark protocol.
