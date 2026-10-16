# Generic references and borrowed receivers, common foundation

Status: CONTRACT FROZEN, October16,2026. Implementation requires separately
scoped explicit GO after this contract is committed. The prior canonical trait
checkpoint is pushed as d0e3b7330 with independently verified1020-case evidence
fa9a61f41. Full independent Rust and existing-project compatibility remain the
objective. All seven owner/independent feasibility reviews cleared the exact
shared architecture. The exact32 sources are stock-classified and independently
rerun as recorded below. This is not native implementation acceptance.

## Purpose and external contract

Extend the existing shared type/facts/region/loan architecture rather than
introducing a method wrapper, name-based permission or second borrowing solver.
Generic direct reference parameters/results and borrowed inherent/trait receivers
are one foundation needed for real Clone/Copy and ordinary library APIs. This
stage does not itself grant canonical Copy/Clone permissions or claim full Rust.

## First source client

Admit standalone shared and mutable reference carriers in generic signatures and
bodies, including &T and &mut T with rigid authoritative caller parameters.
The referent may be an admitted reference-free primitive, tuple, array or nominal
application containing type parameters. Type arguments remain recursively
reference-free. Admitting reference FORMALS must not admit hidden reference
substitutions, nominal fields containing references, nested references or
reference-containing aggregates. Existing nongeneric reference behavior stays.

Admit inherent and required-trait methods with &self and &mut self, including a
named receiver lifetime where the existing canonical region grammar suffices.
The receiver contract distinguishes shared and mutable reference mode, not local
binding mutability. self and mut self retain their owning behavior. No change
to trait method type-parameter, default body, associated type or trait-object
boundaries is implied.

Required trait Self remains genuinely possibly-unsized signature-only metadata.
A reference to Self is a sized carrier without making its referent Sized. This
must not allow by-value Self aggregates, arrays including length zero or nominal
arguments whose formation requires Sized. Trait items still have no body/native
slots. Actual admitted caller type parameters and nominal implementors retain
their existing Sized body contract before lowering or layout.

## Exact authority and reference-result provenance

Keep the immutable canonical trait world and subject-derived private facts.
A generic body receives the exact declaration GenericEnv and assumptions, and a
closed instance proves entry requirements under empty assumptions. Every direct
or abstract call retains all existing live/dead predicate/signature validation.
Required signatures receive schema-only facts, never body or layout permission.

Type substitution is simultaneous and binder-qualified. It rewrites a reference
referent using the same canonical service and inserts caller actuals verbatim.
It does not reinterpret source lifetimes as native instance identity. Region
contracts remain explicit sidecar metadata and must be transported canonically.

CRITICAL: reference-result eligible input paths are declaration-derived authority.
Specializing different symbolic parameters T and U to the same concrete type
must NEVER add newly equal input candidates or result origins. Preserve the
canonical declaration candidate set and path/region provenance while
substituting referent types. A closed validator checks the transported contract
against its exact world subject/schema, not against a newly widened set derived
only from concrete type equality. Missing or forged paths still reject. The
existing result-origin/lifetime-dependency separation and actual candidate
checks remain mandatory.

The exact canonical signature service is
ir::SignatureSubject::{Body,Call(&Callee)} with
TraitCx::validate_signature(subject,&Signature,&ValidatedFacts) returning a
fallible authority result, forwarded by facts. Body identity and actuals derive
ONLY from the exact facts subject; Schema facts cannot validate a body. Call
identity derives ONLY from canonical Direct discharge or TraitItem lookup.
World construction validates rigid declaration completeness once. Closed checks
compare schema-substituted transported contracts, then validate stored paths,
kinds, regions and dependencies without fresh coincident-type enumeration.
Structural fixture RefContract::validate retains full completeness. A private
stored-path checker may omit enumeration only behind canonical subject agreement.
IR/CFG body and every callee, including ownership contract preflight, use this
same service. No caller skip flag or forgeable proof token exists.

Do not erase region contracts, manufacture successful facts or accept arbitrary
caller-provided signature evidence to bypass this specialization distinction.
Source-inaccessible schemas and invalid unused bodies remain all-body gates.

## Receiver regions and evaluation

The source receiver kind adds Borrowed{kind:BorrowKind}, with optional named
receiver lifetime in a separate source sidecar. Shared IR ReceiverMode adds
Borrowed{kind:BorrowKind}; canonical checked argument0 is Ty::Ref(SelfTy), and
its region lives only in RefContract.param_regions[0]. This does not add a new
BorrowedSelf type or put source binding mutability into trait conformance.

The injected checked receiver is parameter0 but remains absent from the explicit
source parameter vector. Canonical reference contracts must include its actual
reference region at position0. They must not reuse the old by-value padding None
for a borrowed receiver. Normalize named and anonymous receiver regions with the
same declared lifetime authority as explicit inputs.

For an elided output with a borrowed receiver, use the receiver region even when
other reference parameters exist. Static/free/by-value functions preserve the
existing input-position elision rule. Explicit output regions and malformed or
stronger implementation lifetime obligations must still be validated, with
alpha-normalized trait/impl contracts and unchanged provenance restrictions.

Select a borrowed method using read-only receiver type/place evidence BEFORE
materializing an owning expression. Never move a non-Copy receiver and later
pretend a borrow preserved it. For admitted places/carriers, evaluate receiver
storage/provenance once and construct one genuine borrow or reborrow as argument0,
then evaluate explicit arguments once in source order. Reference-returning call
receivers must not replay a call or lose their exact carrier authority.

Owned rvalue receiver temporary scopes, arbitrary autoderef chains, nested
reference coercions and new dynamic-place borrows require explicit supported
rules or an honest legal-unsupported boundary, not a guessed pointer relabel.
The exact supported receiver places are existing local-root Field,
ConstantIndex and one Deref, plus a genuine reference-returning Call carrier.
No implicit owned-rvalue temporary, dynamic borrow or autoderef-chain expansion
is implied.

The shared data-only API is
ir::normalize_ref_contract(raw:&RefContract,budget:&PlanBudget)
->Result<Arc<RefContract>,String>. Frontend resolves input/output IDs and elision
before invoking it; normalization grants no facts or signature authority.

Frontend collects named and fresh input regions, receiver first, with a64-region
cap checked before growth, and resolves output receiver priority or free-function
input-position elision. The one shared IR normalizer then computes full outlives
closure INCLUDING unused named mediators, then projects onto used nodes.
Canonical region IDs follow
first use through parameter0 onward and result, independent of spelling or
lifetime declaration order. Preserve alias classes and sorted nonreflexive
closure edges. Borrowed or named NoReference contracts retain canonical input
regions and closure with no result region. Legacy unnamed free NoReference
region erasure stays only as the explicit existing4096-input compatibility path.
New borrowed/named64-region exhaustion is an honest resource boundary.

World type traversal distinguishes TypeActual (recursively reference-free),
signature/body root carrier, referent and Sized aggregate formation. Memo keys
include semantic type-use context as well as canonical node authority and cached
height. Admitting root reference carriers must never admit a reference type
actual, nested carrier or reference-containing nominal argument via a cache hit.

## Receiver loans and genuine bounded two-phase borrowing

This client INCLUDES real reservation/activation in the SAME borrowing analysis
for implicit mutable receiver autoref and implicit mutable function-argument
reborrow. Explicit source &mut and non-call adjustments remain immediately
exclusive. Overloaded compound assignment remains outside this source client.
Existing Read/Write phases are snapshot timing, not loan activation state.

The agreed checked metadata is ir::BorrowTiming::{Immediate,
TwoPhase{activation:ExprId,argument:usize,reason:ImplicitBorrowKind}}, with
ImplicitBorrowKind::{MutableReceiver,MutableArgumentReborrow{carrier:ExprId}},
carried by ExprKind::Borrow. The carrier is the original checked Local mutable
reference, never an explicit Borrow expression; the reserved place is exactly
that carrier root plus Deref. TwoPhase requires a mutable borrow, an exact final Call
expression with call.args[argument] equal to the borrow expression, and a mutable
formal. MutableReceiver additionally requires argument0 and authoritative
borrowed-mutable receiver metadata. The authoritative read-only query is
TraitCx::callee_receiver(callee:&Callee,facts:&ValidatedFacts)
->Result<ReceiverMode,FactsError>, with exact subject/world/ledger validation.
Frontend private per-call pending frames
patch the final Call identity before publication. Nested calls have separate
frames. Explicit borrows and let/assignment/return coercions cannot accidentally
inherit deferred timing.

The agreed CFG metadata is cfg::LoanTiming::{Immediate,
TwoPhase{activation:BlockId,argument:usize}} on LoanSite. A private lowerer map
resolves checked Call identities to final Call terminators before immutable CFG
publication. Raw CFG validates the union of the two legal operational origins:
argument0 with authoritative borrowed-mutable receiver mode and matching referent,
or a local mutable-reference carrier with exactly Deref and matching mutable
formal. It does not pretend to recover source ExprId/reason after lowering.
Checked IR retains the original adjustment provenance; explicit source Borrow
is always Immediate. Both forms use the same validated activation protocol.
There is no runtime activation instruction. Validate every site,
including dead metadata: exactly one compiler-temporary Borrow definition,
exactly one Move use at the designated mutable Call argument, no aliases, other
uses or escape, and reservation dominance of reachable activation. StorageDead
after the call is allowed. A divergent argument may abandon a reservation.
Preserve the existing lowerer's real typed disconnected Call block after a
Return/Break/Continue/diverging argument: it still carries the original typed
argument temporaries and normal Call terminator, with no edge from divergence.
Map activation to that actual dead Call block, never a sentinel, invented runtime
value, Abandoned timing variant or downgrade to Immediate. Unique temporary
Move validation includes dead Call metadata; dominance is required only for an
entry-reachable activation, so abandoned reachable paths need no activation.
Reentrant live-origin forms are initially explicit legal unsupported under the
existing bounded policy, not every site that belongs to a CFG SCC. Reuse the
reachable backedge test: only an SCC-born loan still live through the outgoing
origin/dependency closure and successor live-in is unsupported, whether reserved
or activated. Per-iteration reserve, activate and die before the backedge remains
supported. Mask dead activation bits by live origin/dependency closure and reset
a site's bit on fresh issuance only after rejecting any prior still-live origin.
Divergent abandoned reservations die by liveness, not a guessed lexical end.

The SAME Plan keeps existing origins, targets, liveness and parent dependencies,
and adds metered per-point MAY-ACTIVATED loan state with forward union at joins.
A live unactivated two-phase loan acts shared for reads/shared borrows, but
rejects writes, moves, storage end and overlapping second mutable reservation.
Immediate loans are active immediately. Do not use activation dominance alone:
an activated path can carry a returned reference through a join with a bypass.

At the exact Call terminator, after argument evaluation and before argument
transfer or mutable effects, check activation against ALL currently live
conflicting loans. Exempt only that loan and its existing per-target ancestor
authority. Other reservations activating at the same call are not exempt and
must cross-check. Parent dependency closure does not activate reserved children.
Returned reference origins preserve activated dependencies after the single-use
temporary is consumed. Every state product, traversal and join is charged to the
same ledger before allocation. No lexical EndLoan guesses, argument reordering,
receiver double evaluation or private unchecked native path are alternatives.

## Shared solver, native and resource requirements

Opaque type parameters remain opaque move shapes. Shared reference carriers are
Copy intrinsically, mutable carriers are not, independent of referent shape.
Moving non-Copy values through dereference remains forbidden, and an explicit
Move remains consuming. Generic permission queries use exact validated facts.
No custom trait name can grant Copy or reference/layout authority.

Use the same formal external roots, origin sets, ancestor authority, carrier
liveness, initialization checks, mutable-call variant-proof invalidation and
return-origin substitution. Closed calls resolve traits and discharge direct
requirements before fresh lowering/analysis. Only exact ConcreteCfg native
capabilities reach layout/emission. References remain thin addresses, not copies
of aggregate referents. No change to kernel ABI without measured necessity.

All new region/path/receiver candidate walks, contract comparisons, failed
lookups, implicit loan metadata, caches and scratch use the same snapshot ledger.
Keep32 total type parameters,4096 combined applications,128 depth,1M nodes,
32M work,128MiB shared retained/temporary metadata,4096 shared obligation entries,
256 predicates per environment/header and64KiB rich diagnostic allowance with
only the documented fixed legacy transport exception. Preserve additional
existing per-function path/frame/loan caps, including combined4096 explicit plus
formal origins. Charge before comparisons and allocation, including old/new
vector relocation peaks. No fresh per-method/instance/retry budget or lock held
across recursion/await. Memo keys include exact authority/context and reused
subtree heights must respect depth. Preflight MAY-ACTIVATED point-by-loan-word
products including incoming/outgoing/worklist scratch, activation candidate and
ancestor/liveness scans, SCC edge masks and repeated joins before growth. Keep
existing combined4096 origins including formal and CallReturn rows and1M
memberships. Region normalization charges full closure work and scratch including
unused mediators before computing, not only final projected edges. Abstract
refinement composition charges every stored anchor/path/dependency membership.
Production must not bypass this through legacy unbudgeted RefContract::validate;
use the same bounded authority-aware stored-contract checker behind the world.

## Abstract input-origin envelopes and private implementation refinement

Required fn field(&self)->&i32 and ordinary generic forwarding
fn field<T:Trait>(x:&T)->&i32{x.field()} cannot enumerate a concrete field path
through opaque Self/T. They are admitted by one declaration-derived provenance
contract, not a trait-only exception and not invented opaque layout.

IR adds InputEnvelope{param:usize} and ReturnContract variants
SharedInputEnvelopes(Arc<[InputEnvelope]>) and
MutInputEnvelopes(Arc<[InputEnvelope]>), retaining existing exact InputPaths
variants. At the RIGID declaration, determine lifetime/kind-eligible reference
inputs and the reference-result referent. If ANY eligible input referent OR the
reference-result referent contains an opaque Param, including trait Self,
choose the envelope variant for the WHOLE contract, listing ALL eligible
input indices in canonical input order. Otherwise retain complete existing exact
InputPaths. Eligibility is region/kind based, not result-pointee equality in the
envelope case. This is not an empty-exact-set fallback: mixed abstract and known
return branches need the same guarantee. The variant and indices are transported
unchanged across specialization, even when T/U become equal. No body inference,
layout inspection or post-substitution path reclassification may add authority.

The single data-only derivation API is
ir::derive_return_contract(params:&[Ty],result:&Ty,regions:&RefContract,
budget:&PlanBudget)->Result<ReturnContract,String>. Call it AFTER normalization
on rigid schemas. It ignores the regions.returns placeholder, uses canonical
region closure/input kind, and returns NoReference for a non-reference result.
Frontend derives once, world build validates through the SAME helper, and no
instance rederives it. All opacity/type/candidate walks and output bytes are
bounded and charged before growth. normalize_ref_contract preserves payloads.

The SAME borrowing Target adds private TargetPrecision::{Exact,Subtree}. A
Subtree keeps the selected actual-input canonical ANCHOR forever through result
projections, reborrows and calls. Its returned view is typed by the result
referent, not by pretending the anchor has that type. Never append a returned
view's field coordinates to a differently typed input anchor. Conflicts and
proof invalidation conservatively cover the whole anchor; reads require anchor
initialization. Abstract writes invalidate anchor proofs but do not establish
whole-anchor initialization. Candidate-specific ancestor authority and separate
lifetime dependencies remain unchanged. Runtime pointers still come from actual
call results, never from analysis footprints.

A body proves actual exact paths or abstract anchors lie within the declared
input envelopes, including region/kind constraints. Universal abstract calls
retain conservative Subtree provenance. A private world ReturnRefinementProof
binds exact world, required trait item, implementation callable and both rigid
normalized contracts. It permits trait-envelope to canonical implementation
exact paths, or trait-envelope to implementation envelope with an index subset.
Generic implementation schemas may remain envelopes. Refinement records retain
non-owning exact WorldId plus item/implementation/rigid contract identities,
never an ArcWorld backreference. ResolvedTraitCall retains private refinement
and goal evidence. The private empty-assumption proof context uses the SAME world
ledger/cache and cannot mint a fresh budget or body facts. Implementation body
proofs are still mandatory all-body gates, separate from header refinement.

Generic rigid goals retain abstract calls. An already fully closed concrete
TraitItem goal may use the SAME world resolution and private refinement before
universal body checking, so a concrete wrapper returning a known private field
is not falsely rejected against its own exact declaration contract. Frontend
consumes the one canonical resolved callee/signature, never recomputes refinement
or uses a concrete body success as evidence. All implementation bodies still
must pass the universal all-body gate before native planning.

The single shared pass is
ir::canonicalize_checked_calls(source:&CheckedFunction,facts:&ValidatedFacts,
cx:&mut impl SubstitutionCx,budget:&PlanBudget)->Result<CheckedFunction,FactsError>.
It accepts Universal or Closed facts, never Schema. Universal preserves open
rigid TraitItems but resolves fully closed goals with private empty-assumption
proof. Closed rejects every residual TraitItem. type_callable_with_plan invokes
this before final checked-body publication and lower_template, so the retained
source Arc is the exact canonical published body. Existing close_checked_calls
is a Closed guard delegating to this SAME visitor, not a second algorithm or
analysis. Generic instantiation uses the same closure.

Closing a TraitItem FIRST validates its original canonical signature under
SignatureSubject::Call(TraitItem), then resolves and replaces callee AND signature
TOGETHER with Direct plus the IMPLEMENTATION CANONICAL signature through that
private refinement. This is not raw envelope-vs-exact equality or arbitrary
signature mutation. Later SignatureSubject::Call(Direct) demands that exact
canonical implementation signature. Body/entry contracts remain exact matches
to their own declaration-derived transported schema, which may be an envelope.
Narrowing is permitted only through private authoritative refinement; no concrete
type coincidence can manufacture new origins or candidate exemptions.

## Frozen exact32 oracle inventory

Stock BUILD/CHECK/runtime classification and an independent root rerun both
passed32/32. The immutable source artifact is
`/home/jeremy/.jcode/scratch/generic-references-oracle-verified/corpus.json`, SHA256
`5d13d72b1a7b006e507621c6a43a339ea92344b185655ca0c31113f81d7a38c7`.
All16 positives and4 legal exclusions BUILD/CHECK0 with both artifacts and
runtime0, exact stdout and empty stderr. All12 invalids BUILD/CHECK1, no artifacts
or runtime. No source, full_source, stdout, status or category was corrected.
The root separately recompiled both phases and executed all20 valid cases,
checking full-source hashes/bytes and exact diagnostic-code multiplicities.
Root receipt: `/home/jeremy/.jcode/scratch/generic-references-root-stock32-verified.jsonl`.

The original runner incorrectly required a nonempty error code for N18, whose
real structured stock diagnostic is level:error, code:null, exact message
`lifetime may not live long enough`. Root independently verified that message in
both phases with exit1/no artifacts. The sole authorized correction was N18
expected/message METADATA plus runner acceptance, not source or classification.
The corrected sibling retains ALL31 other dictionaries unchanged and ALL32
source/full_source/stdout/status/group/hash values unchanged. The original
`generic-references-oracle` artifacts remain byte-for-byte preserved:
initial SHA `7fafaadea9417e648389ce37069a34fc6896206b046e44c0cf9fe4abe4e0057e`,
corpus SHA `4da32837e0942b458a76a7c1db9fc2468922a736515c97b11f8ef2c3c872ae00`,
results SHA `0f6ac5adb2584110efaee584db03c62a2d69c57740d3ee6fbe54b490da52eb38`.
Sibling runner-fix record SHA
`d927c2d2b585718d33ae0670f4153f7fe278f1f0b0b9ab696c4f496b4a07fb84`.

Observed invalid codes, identical in BUILD/CHECK:
N17 E0507; N18 the exact code-less lifetime diagnostic; N19 E0506; N20 E0053;
N21 E0502; N22 E0503; N23 E0506; N24 E0505; N25 E0499; N26 E0502;
N27 E0506; N28 E0597. The future stock-only harness must check N18's actual
structured message, not accept empty stderr or a compiler crash. No native
acceptance, project fixture publication, historical promotion or growth is
implied by this stock freeze.

Positive16:
01 generic_shared_identity_and_rigid_forwarding
02 generic_mutable_identity_and_write
03 shared_carrier_copy_noncopy_referent
04 reference_free_product_referents_and_disjoint_fields
05 T_U_coincidence_preserves_declared_region_authority
06 inherent_shared_receiver_preserves_owner
07 inherent_mut_receiver_write
08 receiver_output_elision_over_other_reference_input
09 named_receiver_alpha_normalized_contract
10 trait_projected_shared_mutable_returns_and_generic_forwarding
11 conditional_trait_dispatch_and_static_trait_result_opacity
12 genuine_reference_call_receiver_once
13 receiver_and_explicit_arguments_once_ltr
14 implicit_mut_receiver_shared_read_then_activation
15 implicit_mut_argument_reborrow_shared_read
16 returned_mut_ref_last_use_and_per_iteration_two_phase_loop

Invalid12:
17 unused_generic_body_move_through_deref
18 wrong_result_region_despite_T_U_concrete_coincidence
19 mutable_reborrow_parent_conflict_live_child
20 unused_impl_borrowed_receiver_kind_mismatch
21 explicit_mut_receiver_immediate_conflict
22 explicit_mut_argument_immediate_conflict
23 reservation_blocks_argument_write
24 reservation_blocks_argument_move
25 overlapping_second_mutable_reservation
26 shared_argument_loan_live_at_activation
27 returned_ref_keeps_activated_loan_live
28 borrowed_result_owner_storage_dead

Legal excluded4:
29 reference_type_actual_and_hidden_reference_nominal
30 nested_reference_or_reference_aggregate
31 owned_rvalue_borrowed_receiver_temporary
32 dynamic_index_borrow_or_autoderef_chain

## Exact migration ownership and acceptance

Frontend owns source receiver grammar, canonical lifetime normalization,
read-only method selection and once-only receiver adjustment. IR/CFG owns shared
receiver/loan metadata, capture-free metadata substitution and authoritative
transported-contract validation. The world owns signature-only Self formation,
conformance, facts and dispatch proof authority. Ownership/borrowing owns the
SAME solver integration. Native owns thin-address and sealed-call regressions.
Coordinator owns root all-body/instance gates, stable keys, graph acceptance and
retained exact-revision evidence. Version integrated semantics/cache keys for
changed receiver/region/loan contracts, keeping source lifetime spelling and
transient expression/block IDs out of persistent semantic identity. Exact
method/type spellings above are agreed and are not permission to publish
competing APIs.

Before GO read this whole final contract, independently classify a fixed bounded
stock BUILD/CHECK/runtime corpus, preserve all source/stdout/hash/diagnostic
receipts and distinguish invalid Rust from legal exclusions. Cover unused
invalid generic/trait bodies and headers, rigid caller parameters, T/U concrete
coincidence, receiver result elision, explicit lifetime contracts, mutable
reborrows, disjoint fields, conflict/last-use, receiver/argument effects once,
conditional trait dispatch and exact closed world/facts/ledger seals.

Publish production plus all existing fixture migration checkpoints before at
most the approved focused test groups. Preserve every historical source and
assertion. Run hosted full check/test/strict all-target lint/fmt, actual native
examples, independent actual-source soundness/resource review and cumulative
exact-revision native acceptance. Export selected path-redacted profiles,
append findings without deleting history, commit and push working improvements
and raw evidence separately. Never install the experimental package or claim
full Rust, standard-library compatibility or a speedup from bounded cases.
