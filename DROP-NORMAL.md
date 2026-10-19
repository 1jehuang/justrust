# DROP-NORMAL: frozen semantic implementation contract

Date: 2026-10-18. Status: version 1 implemented and accepted for its bounded slice (results below). Full Rust Drop remains open.

## Outcome and non-goals

Deliver genuine source `impl Drop for S { fn drop(&mut self) { ... } }` execution on normal paths for a deliberately bounded nongeneric struct slice. Supported values are destroyed exactly once if still initialized, with observable ordering confirmed by stock-first exact source fixtures. The claim is normal-path destruction with aborting panic, NOT full Rust Drop, unwind cleanup, generic dropck, aggregate glue, or standard-library `drop`/`forget` support.

Canonical Drop/DropItem already exist in `src/compiler/traits/catalog.rs:59-100,297-451`. The immediate admission blocker is `src/compiler/traits.rs:1316-1327`, not missing catalog syntax. Existing Copy/Drop exclusion at `traits.rs:4088-4115` must remain.

## Source admission: version 1

Admit a local nongeneric named-field struct with only scalar fields already supported by the compiler, no references, no nested aggregate fields, and one valid canonical Drop impl. Fields may be inspected and mutated through `&mut self`; the destructor may use supported arithmetic/control flow, static literal output, and validated nonescaping calls. Destructor body and all reachable callees receive ordinary type, ownership, region, budget and call checks. Any broader body form requires the same evidence as other source functions, not a Drop-specific bypass.

Admit owned values in whole local roots and by-value parameters. Admit construction, whole-root moves to another local/call argument/return/break result, delayed initialization, whole-local replacement, ordinary block exit, conditional branches, early and implicit return, loops, labeled break and continue. Admitted functions can remain nongeneric initially. A generic function instantiated with an admitted Drop owner is rejected unless universal drop obligations and concrete elaboration are explicitly implemented; never silently treat its parameter as no-drop.

Fail closed for: field moves out of a Drop owner including patterns; enum/tuple/array containers containing Drop values; nested Drop fields; reference-containing Drop owners; indirect, indexed or borrowed destination replacement; destructuring of Drop owners; match/if-let/let-else involving owned Drop values; generic Drop owners/impls/functions carrying Drop values; closures, unsafe, FFI and unwind cleanup. Reject aggregate wrapping of the owner even if only one field currently needs Drop. This prevents accidental leaks through existing generic/aggregate syntax. Ordinary unrelated values and control flow remain admitted where they cannot carry an owner outside this slice.

Scalar Copy field reads and writes through the genuine `&mut Self` destructor receiver remain admitted, including historical `let result = s.v` cases. A field projection is not itself an ownership move. Reject only actual non-Copy field moves and unsupported destructuring obligations, not all projections.

Direct source invocation of canonical `.drop()` and `Drop::drop` must be rejected. Compiler-generated cleanup is separately authorized. A user-defined trait with the spelling `Drop` is not canonical and gains no destructor semantics.

## Temporary strategy: make supported lifetimes explicit

Existing `Lowerer.scopes: Vec<Vec<LocalId>>` conflates storage and semantic destruction scopes. A wholesale conversion of every `StorageDead` to a destructor is wrong.

Version 1 records separate semantic drop ownership for roots and materialized expression temporaries:

* Local bindings live until the enclosing lexical scope exit, in reverse binding order. Parameters are dropped on function exit in reverse parameter order after locals.
* A temporary owned value used by an expression statement is destroyed at the end of that statement if not moved away. This includes an ignored non-place construction or by-value call result.
* Internal snapshots used to compute a let initializer, assignment RHS, call arguments, return operand or break operand transfer their initialized flag to the destination; they cannot become extra independent owners. Unmoved statement temporaries end at that statement. Call-argument temporaries remain owned until successful argument transfer, then the callee owns parameter cleanup. On a normal early return/break while evaluating a later argument, already-constructed earlier arguments must be cleaned by their caller-side scope. Such abrupt argument forms must fail closed if not implemented; do not merely test calls whose arguments all complete.
* Return and break values must be moved into the protected destination before inner cleanup. The return root is excluded from function cleanup; the enclosing loop/block result root survives the exited scopes.
* The first version admits only temporary forms for which these scopes are explicit and stock-confirmed. Block tail temporary extension, borrow-based temporary lifetime extension, pattern temporary scopes, condition chains and newly discovered edition-sensitive cases must be rejected if not implemented. Do not reject ordinary primitive conditions merely because a Drop local exists nearby.
* `let _ = existing_place` must not spuriously consume or drop that place. An ignored constructed owner still requires end-of-statement destruction. If those forms cannot be distinguished safely, reject the affected Drop-bearing form.

All manual StorageDead sites in cfg.rs, including match/let-else and condition scope handling, need review for interaction even if Drop-bearing operands there are rejected. A static unsupported-source check must cover dead syntax too, consistent with existing proof policy.

## Proposed authoritative APIs (names illustrative, contracts mandatory)

### 1. Trait facts and admission

`ValidatedFacts::normal_drop_plan(&Ty) -> Result<NormalDropPlan, FactsError>`

`NormalDropPlan = Trivial | Destructor(ValidatedDestructor)`; unsupported shapes return Unsupported, not Trivial. Resource exhaustion and failed proof are errors, never absence. `ValidatedDestructor` has private construction and binds canonical nominal owner, canonical Drop trait/item identity, direct concrete CallTarget, exact `(&mut S) -> ()` signature, world, snapshot, facts phase, and shared PlanBudget authority. It is not a caller-supplied function pointer or a lookup by method spelling. Query results may be cached only under the exact type/world authority.

A structural `contains_normal_drop`/admission walk must reject an admitted destructor hidden in unsupported aggregate or generic positions. It must distinguish actual absence of canonical Drop from unsupported or unproved generic facts. No generic negative assumption is permitted.

### 2. Semantic cleanup input

Lowering supplies immutable `DropScopeId`, ordered droppable root registrations, and exit obligations. A registration identifies root, type, owning scope, declaration/temporary order and origin. An exit identifies source block and target continuation or Return, exited scopes, protected result root and source origin. Replacement explicitly identifies the old destination and already-evaluated RHS root.

These are semantic data, not inferences from final frame offsets. Initializing stores, consuming moves, overwrite stores and compiler-internal snapshots must have distinguishable ownership effects. Existing plain Assign is insufficient as the entire drop contract.

### 3. Exact elaboration evidence

Preferred implementation seam: canonical source lowering incorporates a deterministic cleanup-elaboration stage before publishing the final checked CFG. Universal admission rejects unsupported generic Drop cases first. If elaboration is a separate graph transform, return an opaque `ElaboratedDropCfg` proof binding original checked body, original lowering proof, exact input and output CFG, semantic drop plan, canonical facts, target dependencies and budget. It must be created only by the elaborator, never from a caller-provided success bit.

`GeneratedCfgShape` currently compares exact operations, locals, edges, types, capsules and loan timing. Keep that guarantee. Either capture its final canonical shape after the integrated elaboration or explicitly validate the transform proof at each wrapper boundary. Do not mutate `ClosedCfg`/`ConcreteCfg` after analysis. `ConcreteCfg` must certify the graph actually emitted.

### 4. Synthetic destructor calls

Provide `ValidatedCleanupCall` distinct from source `ValidatedCallEvidence`. It binds: validated destructor, owner Place/type, generated exclusive borrow identity, cleanup site, direct target and signature, exact output CFG, origin and PlanBudget. Only the cleanup builder creates this evidence. It cannot authorize arbitrary source calls or bypass ordinary signature/noescape checks.

Normal calls keep their existing source publication/capsule invariants. Do not fabricate an ExprId or source call capsule to make an implicit destructor fit. Extend the canonical call-evidence sum/validation path deliberately where required.

Preferred concrete representation, refined with the implementation lead: `Terminator::Drop { site: DropSiteId, next: BlockId }`, backed by privately constructed `ValidatedCleanupCall` and exact owner root/destructor metadata. This atomic operation conditionally guards the runtime eligibility flag, clears it before invocation, and reuses the existing direct-call `&mut S` ABI. An ordinary synthetic Call plus explicit guard blocks remains an allowed alternative with equivalent proof. The atomic representation avoids requiring general boolean-edge refinement to prove initialization.

Ownership treats the atomic operation as a conditional exclusive receiver access when the root may be initialized, not an unconditional definite-initialization read. On the taken path the object remains readable throughout the destructor invocation; afterward it is uninitialized. The skipped path remains uninitialized. Eligibility must be distinct from readable initialization during the call. Conditional access must preserve actual live-loan conflicts; absent correlation evidence, conservative rejection is safer than erasing conflicts.

For replacement, Drop leaves storage live for the new value. For scope exit, a following storage end closes the lifetime. The exact operation/site/target/signature/root/flag relationship must participate in canonical graph shape validation, resource walks, call-target discovery, effect/noescape checks and mutation tests. Backend ABI reuse is not permission to bypass ordinary analyzed target requirements.

### 5. Implicit target closure

Elaboration publishes canonical direct destructor targets to the existing executable instance-demand closure (`instances.rs:517ff`). The closure deduplicates by the same exact identity as ordinary calls, analyzes each destructor body, follows its ordinary/implicit callees, and charges the existing budget. No backend symbol lookup may synthesize a missing body. The backend must fail if the target/evidence is absent or differs from the plan. Recursion is handled by the existing finite demand mechanism, never unbounded inlining.

## Runtime flag and transfer contract

One boolean initialized/drop-needed flag per admitted owning root is the straightforward initial representation. Optimize it away only when proven equivalent. Distinguish runtime destruction eligibility from ordinary readable initialization when entering a destructor.

1. Nonparameter root begins false on StorageLive. By-value initialized parameter begins true at entry.
2. Successful construction or incoming move initializes destination and sets true.
3. Whole-value consuming move clears the source flag and transfers bytes/ownership exactly once. Backend byte copying alone does not duplicate ownership.
4. Scope cleanup branches on the flag. If false, do not read the root or call Drop. If true, clear flag before destructor invocation, keep storage available for receiver reads, call destructor, then end storage at the appropriate exit.
5. Assignment fully evaluates and owns RHS in a protected temporary before destroying old destination. If old destination flag is true, drop old value; then move RHS into destination and set true. If RHS evaluation diverges by abort, no normal continuation cleanup is promised. For `x = x`, RHS move makes old root ineligible, so do not destroy the moved value prematurely. Assignment that reinitializes a previously moved/uninitialized root must not attempt old-value destruction.
6. Branch joins retain real runtime flags. `must_init` alone cannot choose whether to drop at a join. Loops initialize/reset flags on each storage incarnation; continue cleans iteration scopes and break cleans exactly exited scopes.
7. Return/break result snapshots are protected before cleanup; drop only at the receiving owner's eventual scope. Call argument ownership transfers to the callee, whose parameter cleanup owns destruction.
8. Cleanup call mutable accesses are checked against live loans. Existing StorageEnd checks do not prove destructor safety by themselves. Unsupported contained-reference/dropck cases stay rejected.

## Existing implementation touchpoints

| Concern | Current location | Required change |
|---|---|---|
| Canonical identity/signature | traits/catalog.rs:59-100,297-451 | Reuse, do not duplicate |
| Admission and Copy exclusion | traits.rs:1316-1327;4088-4115 | Replace global rejection with bounded admission; retain exclusion |
| IR storage and plain assignment | cfg.rs:421-427 | Add semantic ownership effects/cleanup representation |
| Scope registration and cleanup | cfg.rs:2340;2910-2929 | Separate drop scope from physical storage |
| Assignment/return | cfg.rs:3159-3193 | RHS protection, replacement cleanup, protected return |
| Tail/function cleanup | cfg.rs:3244-3261;4163-4179 | Protected results and parameter cleanup |
| Break/continue | cfg.rs:3721-3758 | Cleanup exact exited scopes |
| Manual scope ends | cfg.rs:2675,2688,2857,2867,3228,3235 | Audit; integrate or reject Drop-bearing forms |
| Definite initialization | ownership.rs:1693,2007,2586-2672 | Validate flag/move/replacement obligations and generated calls |
| Exact graph proof | cfg.rs:66-229 | Preserve exact canonical shape or explicit transform proof |
| Successful wrappers | ownership.rs:301,409,513 | Analyze emitted graph; no post-seal mutation |
| Borrow effects | borrow.rs storage visitors around2454,2985,3806 | Model cleanup receiver access and liveness |
| Native execution | backend.rs:1670,1788ff,1847ff | Reuse direct calls; do not make StorageDead magically call |
| Demanded targets | instances.rs:517ff | Include implicit destructor bodies |

Line numbers are from read-only inspection of a concurrently modified tree and are navigation hints, not immutable revision identifiers.

## Requirements matrix: freeze BEFORE implementation

All rows are required for this claimed slice unless explicitly rejected. Write exact sources and stock receipts first. Destructors use distinct static literal println markers to avoid formatting or mutable-global dependencies.

| ID | Requirement | Stock-first positive/negative fixture and expected check |
|---|---|---|
| D01 | Canonical source admission | Real canonical impl accepted; ordinary user trait named Drop gets no automatic calls |
| D02 | Exactly-once normal exit | One initialized local prints its destructor marker once |
| D03 | Lexical order | Two distinct owner types plus nested blocks prove reverse local binding order |
| D04 | Parameter order | Two by-value owner parameters plus local prove parameter cleanup after locals and correct reverse order |
| D05 | Early/fallthrough return | Same owners on both forms, same correct cleanup before caller marker |
| D06 | Returned ownership | Returned owner survives callee cleanup and is dropped only by caller |
| D07 | Move/call transfer | Local-to-local and by-value argument moves each produce one destructor invocation; normal early exit while evaluating a later argument cleans earlier temporary arguments, or this exact form is explicitly unsupported |
| D08 | Conditional initialization | Initialized on one branch only; drop iff initialized on both executions |
| D09 | Conditional move | Move on one branch, retain on other; no double drop at merge |
| D10 | Replacement | RHS construction marker before old-drop marker; new owner drops at eventual exit |
| D11 | Reinitialization/self-move | Moved-out root assignment skips old drop; `x=x` preserves single final destruction |
| D12 | Loop lifetime | Repeated iteration-local construction/continue drops once per iteration, flags reset |
| D13 | Labeled exits | Nested break/continue drop only exited locals and retain enclosing owner |
| D14 | Break payload | Owned payload survives cleanup and drops in receiver scope |
| D15 | Statement temporary | Ignored construction/call result drops before next statement marker |
| D16 | Ignored existing place | `let _ = x` does not accidentally move or drop x |
| D17 | Drop-owner field moves | Non-Copy field move/pattern rejected before emission; scalar Copy field reads remain legal |
| D18 | Explicit destructor call | Dot and UFCS canonical drop calls rejected, source lookalike trait unaffected |
| D19 | Copy exclusion | Existing Copy+Drop fixture remains invalid before unsupported diagnosis |
| D20 | Unsupported propagation | Generic/aggregate/ref-containing/indexed paths fail closed, including otherwise dead syntax |
| D21 | Borrow boundary | Owner cannot be destroyed while a later-use reference requires it; generated cleanup accesses checked |
| D22 | Abort boundary | If included, panic fixture compared with stock panic=abort; no unwind cleanup claim |
| D23 | Proof integrity | Mutated/removed cleanup, changed target/signature/world/flag, missing implicit body and stale proof rejected |
| D24 | Resource budget | Oversized scope/target/cleanup expansion fails closed without partial proof publication |

For D17 the version-1 scalar-field owner cannot itself exercise a non-Copy field. Use a broader rejected owner shape fixture or separately rejected nested owner to ensure projection moves cannot bypass the gate. Do not label scalar field copies as prohibited moves.

## Oracle and implementation gates

1. Semantic D01-D24 version 1 approved by coordinator on 2026-10-18; concrete admitted source fixtures and stock/native receipts remain pending.
2. Stock-classify exact source text in scratch using installed stock rustc. Retain `rustc -Vv`, edition, flags, source hash, compiler status and bounded runtime stdout/stderr/status. No stock invocation has yet happened for this document.
3. Keep supported positive fixtures distinct from deliberate unsupported cases. Rust-valid but unsupported cases must receive honest unsupported diagnostics, not be mislabeled invalid Rust.
4. Add source-to-native CLI comparisons using repository patterns (`tests/labels.rs`, `tests/leading_zeros.rs`); update formerly unsupported canonical case at `tests/canonical_copy.rs:535` only after positive execution exists. Preserve negative at :435.
5. Add CFG and proof-authority adversarial tests before claiming sealed integration. Validate every emitted destructor has a demanded analyzed target.
6. Run stable justrust check/test/clippy and cargo fmt through the coordinator after implementation, never install the experimental compiler over stable tooling. The document owner remains review-only and has run none of these.

## Independent review checklist

* Source/receipt linkage: exact source hash, stock version/edition/flags and expected output travel together. A success exit with missing destructor markers is a failure, not a pass.
* Conditional cleanup: run both outcomes of each move/init branch. Confirm definitely-uninitialized paths neither read object bytes nor invoke its destructor. Test a join with may-init but not must-init.
* Runtime flag integrity: source move clears the correct root; incoming move initializes only the destination; loop StorageLive resets flags. Instrument/mutate lowered graphs to catch stale flag reuse, double clear and missing set.
* Atomic Drop proof: forge site/root/type/target/signature/continuation/world/flag identity individually; delete a Drop or append one after sealing. All altered graphs must fail authority checks. Private site construction alone is not enough if mutable public graph metadata can bypass equality checks.
* Initialization during calls: destructor can read and mutate its scalar fields even though its drop-eligibility flag has been cleared. After replacement Drop, destination storage stays live but its old value is uninitialized until the incoming write.
* Borrow checks: test immutable and mutable references with later uses across destruction, including conditional cleanup paths. Do not silently omit effect/noescape participation because Drop is not an ordinary Call terminator.
* Target closure: a destructor not explicitly called in source is still emitted exactly once per demanded target, receives normal analysis, and contributes its own callees. Missing target or failed body cannot pass via a backend-only symbol shortcut.
* Scope/order: check return/break result transport before inner cleanup, params after locals, self-move replacement, RHS side effects before old cleanup, and prior argument temporaries on abrupt later argument evaluation.
* Unsupported shape closure: generic/tuple/array/enum/wrapper/reference-bearing routes and dead syntax cannot hide a Drop owner from admission. Distinguish unsupported Rust-valid input from language-invalid input.
* Regression gate: preserve historical scalar field reads/writes, canonical versus shadow Drop identity, Copy+Drop rejection, ordinary source call capsule checks and existing non-Drop reference/loop behavior.

## Source allocator extension (2026-10-19)

The ordinary embedded `Allocation` now implements canonical source Drop using
existing cleanup authorization and runtime flags. Raw pointers are nonowning
scalar fields, admitted alongside integers in both the trait-world and exact
normal-drop descriptor gates. The blanket reference-to-owner exclusion is
removed: ordinary shared/mutable borrows use the same source loan analysis and
the cleanup operation's exclusive receiver access, not a library exemption.
The previously frozen `unsupported/borrow.rs` fixture is now native-positive;
its original stock receipt and source remain unchanged (27 native positives).

`tests/heap.rs` checks mapping presence/absence across returned/moved owners and
early returns, explicit-release disarming, raw-pointer destructor exact-once
counters for conditional moves/loop continue/replacement/self-move, shared and
mutable owner helpers, and stock/native borrow escape, move and replacement
conflicts. A panic=abort control requires no user cleanup output. Generic and
aggregate owners, partial moves of non-Copy fields, nested owning fields and
unwinding remain excluded. Generic functions with these nongeneric owning
values are supported by the concrete-cleanup extension below. This does not
implement Box<T> or generic owning structs.

## Concrete generic-function cleanup (2026-10-19)

Universal source proof remains authoritative while each demanded closed
instance is re-lowered before capturing its generated CFG shape. Generic
functions can now carry the supported nongeneric owner. The instance frontier
resolves each concrete canonical destructor and retains its implicit demand;
ordinary analysis and object graph dependencies remain mandatory. Universal
unresolved Drop never becomes evidence of trivial executable destruction.

An adversarial unit starts with a generic universal template containing no
Drop, instantiates it with a scalar-field owner and requires concrete cleanup.
Deleting that cleanup, transplanting the universal lowering shape or removing
its destructor demand must fail. Stock/native whole-owner tests require the
exact counter value eight across transfer, conditional consumption, early
return and replacement. Real mmap controls cover generic return/break transport,
parameter cleanup and consuming release without double-unmap. Generic move and
borrow conflicts remain semantic errors; tuples/arrays containing owners still
reject explicitly. Frozen `unsupported/generic_function.rs` and
`unsupported/dead_generic.rs` sources and receipts are unchanged and promoted
to native-positive (29 native positives).

## Next prerequisite: generic owners and pointee destruction

Then admit generic pointer-field owners through canonical trait resolution of
closed owner arguments. Layout and `drop_in_place` need typed graph facts keyed
by the closed type, never process-global reflection or unvalidated backend
calls. A source `Box<T>` must destroy its pointee before freeing its allocation,
transfer/disarm allocation ownership exactly once, and preserve nested-owner,
projection, partial-move, borrow and abort controls. A Copy-only or manually
released container is not completion of this prerequisite. Exact Deref remains
separate until its associated `Target: ?Sized` contract is supported.

## Approved integration constraints; concrete API refinement

Prefer integrating semantic drop elaboration inside the canonical lowering publication, so existing exact shape proof naturally captures the final graph. If source-call replay makes synthetic calls impossible there, add an explicit cleanup evidence variant rather than weakening replay. Assign one owner to shared cfg/ir/ownership proof interfaces. Traits/instance discovery and oracle work can proceed in parallel only after those types are agreed.

## Acceptance results (2026-10-18)

Stock-first corpus `tests/fixtures/drop_normal/receipts-v9`: 56 sources frozen and executed with stock rustc 1.98.1 before native expectations were set. Native classification, enforced by `tests/drop_normal.rs` through public CLI and observed CHECK/BUILD at workers 1 and 8:

- **25 native-positive**: compile and run with byte-identical stdout to stock, in bootstrap debug and release profiles. Covers D02-D16 including declaration vs initialization order, parameter-after-local order, early/fallthrough return, returned and moved ownership, conditional init/move joins, RHS-first replacement and self-move, loop flag reset after a moved iteration, labeled break/continue, break payloads, statement/binding/wildcard temporaries, `if`/`&&`/`||`/`while` condition temporaries, interrupted call arguments, destructor helper chains and observable destructor mutation.
- **4 semantic rejections** (D18/D19): manual Copy+Drop (E0184), `.drop()` method, concrete `<D as Drop>::drop`, and unused generic `<T as Drop>::drop` (E0040). The last is rejected before the no-destructor fast path.
- **27 explicit boundaries**, never counted as conformance: 16 legal unsupported owner/position shapes with Drop-specific unsupported diagnostics (generic, nested, array, enum, tuple, borrowed, reference field, dead aggregate/generic, indexed/borrowed replacement, owned pattern/match/if-let/let-else), and 11 sources blocked earlier by unrelated grammar (`#![allow]`, `#[derive]`, unqualified `Trait::method` paths, `mut` by-value parameters, `String::from`, nominal lifetime parameters). Each has a separately frozen in-grammar variant where meaningful.

Defects found and fixed during acceptance: `if` conditions and lazy-boolean left operands held Drop temporaries past the branch (edition 2024 drops them first), an ignored dynamic-index statement leaked a temporary scope, explicit generic destructor calls bypassed rejection when no Drop impl existed, and the no-destructor fast path initially regressed slice validation. Each fix has a frozen stock case or internal regression test.

Internal controls in `src/compiler/{cfg,traits}/normal_drop*/tests.rs` cover cleanup omission, owner transplant, Move/Copy substitution, stale proof, foreign world, StorageLive deletion, Drop successor tamper, canonical target/signature corruption, removed instance demand, live field loan vs certified cleanup, and real budget exhaustion with no CFG publication.

D17 is partial: non-Copy field-move conformance is blocked by the excluded owner shape. D22 is not claimed: no unwind cleanup. The historical canonical-copy X32 witness is promoted from legal-excluded to executed. Two panic controls now expect the explicit unit-struct owner boundary instead of the removed blanket Drop rejection.
