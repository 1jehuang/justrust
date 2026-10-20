# Associated declarations and inherent value dispatch

Status: proposed architecture and first implementation contract. Not implementation GO.
This is one step toward complete Rust support, not a claim of full methods or traits.
Owner API agreement, independent review and a frozen stock-classified corpus must
precede implementation.

## Common foundation, first dispatch client

Build an authoritative associated-declaration and generic-scope foundation, not a
source-to-free-function spelling rewrite. Inherent associated functions and
by-value receiver methods on local nominal structs/enums are the first client.
Real trait declarations, predicates, coherence and proof-aware Copy permissions
are the next client of this foundation. Do not accept trait syntax or manufacture
trait evidence in this change.

Proposed admitted first slice:

- Inherent impl blocks whose self type is an application of a local nominal
  struct or enum, including reference-free generic applications.
- Associated functions without a receiver and methods with `self` or `mut self`.
- Separate impl and method type parameters with owner-qualified identities.
- Explicit ordered method type arguments. Impl arguments for dot calls are
  determined by matching the already checked receiver type against the impl
  self pattern, not by guessing a native layout or a concrete witness.
- Qualified associated calls and dot calls, with canonical resolved call targets.
- Scoped `Self` in admitted type/constructor/associated paths, subject to the
  same canonical application and privacy rules as the spelled nominal type.
- Existing function bodies, scalar operations, moves, field projections, matching,
  calls and native ABIs under the existing language boundaries.

No traits, bounds, Copy/Clone shortcut, derive, associated types/constants,
primitive or foreign-type impls, generic references, receiver autoref/autoderef,
`&self`/`&mut self`, two-phase borrowing, arbitrary typed receivers, or method
function values are implied. Omitted method generic arguments and partial `_`
arguments remain explicit inference boundaries initially. Existing free-function
inference and existing nongeneric references must not regress.

## Declaration authority and source bodies

Add structured impl and associated-item AST and registry entries. Associated
items are not module functions with an invented string name. The registry binds
an associated declaration to its impl declaration, local nominal declaration,
method name, receiver kind, parameter scopes and symbolic signature. Diagnostic
names are never identity.

Source body enumeration must include every associated body exactly once. Root
graph slots, BODY/universal facts, instance planning, statistics and compatibility
entrypoints must use one authoritative callable-body enumeration. All unused
methods and all impl headers are checked. A bad generic method is not accepted
because only a favorable concrete specialization is demanded.

Keep type definition ownership separate from impl-site ownership. Field privacy
uses the actual method definition module, not an automatic privilege granted by
implementing a nominal type. Method visibility is checked through the resolver
and registry, not omitted after method selection.

## Binders and capture-free substitution

Impl parameters and method parameters have distinct declaration-qualified
binders. Reusing a name across unrelated declarations must not alias identities.
Duplicate parameter names in one visible impl/method scope must follow Rust
diagnostics, not become an invented shadowing feature. The checked environment
must authoritatively describe all scopes visible in the associated body. Known
caller parameters are rigid, including recursive calls in the same method.

Do not reuse a method's parameter index for an impl parameter. Do not substitute
by spelling. A canonical call target may encode ordered impl-then-method actual
arguments only when the registry supplies the authoritative scope split.
Universal validation and typed substitution must cover all those scopes across
signatures, local and expression types, nominal applications, calls, variants,
patterns and projected place metadata. A single simultaneous substitution must
not recursively capture parameters inside caller-provided actual arguments.

The shared IR API is `GenericEnv`, privately constructed from ordered
`Arc<[Arc<ParamEnv>]>` scopes, impl first then method. It exposes `scopes`,
`contains`, `total_arity`, `is_generic` and ordered parameter traversal. Retain
zero-arity rows to preserve scope authority, reject duplicate binder rows, and
limit the TOTAL visible arity to32. Free functions use one scope.
`GenericSignature.env`, `FactsCtx::Universal` and exact template proof wrappers
carry this same environment. Genericity means `total_arity > 0`, not a method
syntax node having its own type parameters.

`substitute_ty(ty, env, ordered_args, caller, cx, budget)` performs one
simultaneous owner-qualified substitution across the authoritative scope split.
`instantiate_checked` still requires the exact verified-template capability.
Neither function recursively substitutes into an inserted actual argument.

`Self` is the resolved impl self type in the current authoritative environment,
not a string alias substituted into source. Reject unconstrained impl parameters
under the admitted self-pattern rules. Do not reject legal unused method type
parameters merely because they do not occur in the result or body.

## Applicability and coherence

Impl matching compares nominal source declaration and ordered semantic type
arguments. It does not compare fields, current application IDs alone, display
names or layout. Matching is bounded and capture-free.

Before body facts or native success, validate duplicate items within an impl and
conflicting same-named inherent items across applicable impls. Generic and
concrete overlapping self patterns must not be resolved by declaration order or
choose-first dispatch. Disjoint impls may provide the same method name. Separate
impls with disjoint method names are not intrinsically an error.

The admitted self-pattern grammar is finite reference-free first-order type
terms: existing primitive integers, Bool, Unit, tuples, fixed-length arrays,
local nominal applications with ordered arguments, and impl parameter leaves.
The outermost type must be a local nominal application. Every impl parameter
must occur in that self pattern, including through zero-length array elements.

Selection is one-way matching with only the candidate impl parameters flexible.
The receiver type, including any caller parameters, is rigid. Repeated impl
variables must match the same semantic type. Overlap is a separate two-sided
existential unification after freshening both impl scopes, with exact primitive
kinds, tuple arities, array lengths and nominal declaration identities, plus an
occurs check. For example `Pair<T,T>` is disjoint from `Pair<i32,u8>`, while
`Pair<T,i32>` overlaps `Pair<u8,U>`. Do not confuse failed one-way selection with
proof that two impl patterns can never overlap.

Check same-name item intersections across all overlapping impls, including
unused ones, before body/native success. Charge rejected candidates and pairs.
Resource exhaustion or unrepresented pattern forms are explicit boundaries,
never permission to accept incoherence or choose an arbitrary candidate.

## Shared API ownership and callable enumeration

IR/CFG owner publishes `GenericEnv`, multi-scope facts validation and substitution,
and the proof-wrapper environment migration. `DeclKind` gains real impl and
associated-function kinds. Existing canonical `CallTarget` remains the direct
dispatch representation with registry-ordered impl-then-method arguments.

Frontend owns structured source impl/item/method-call AST, `ReceiverKind`, the
authoritative associated registry, scoped `Self`, selection and overlap checks.
`PlanCtx` exposes these fallible declaration-first APIs:

```text
callable_count(&self) -> usize
callable_decl(&self, index: usize) -> Result<DeclId, String>
callable_index(&self, declaration: DeclId) -> Result<usize, String>
callable_body<'p>(&self, program: &'p Program, declaration: DeclId) -> Result<&'p Function, String>
callable_env(&self, declaration: DeclId) -> Result<Arc<GenericEnv>, String>
callable_context(&self, declaration: DeclId) -> Result<CallableContext, String>
frontend::type_callable_with_plan(program, declaration, plan, budget) -> Result<CheckedFunction, String>
```

`CallableContext` contains definition-module components, optional canonical
`Self` type, optional impl declaration owner, and `ReceiverKind::None` or
`Value { mutable }`. The source Function payload retains only explicit source
parameters. Checker injects the checked `self` parameter at position0 using
this metadata, not by rewriting or re-parsing the source function.

`DeclSemanticPath` gains optional `AssociatedSemanticOwner` containing nominal
declaration, impl declaration, canonical self pattern and receiver kind. IDs
are lookup authority only. Root encodes semantic paths and alpha-normalized
patterns, not the raw IDs. Frontend privately owns `select_associated` and
`validate_inherent_overlaps`; the latter runs during plan-context construction
before body facts can publish. Ordinary free-function compatibility helpers
delegate to the same callable APIs, not a second method compiler.

Root owns common callable graph slots, all-body gates, instance roots and stable
associated symbol encoding. Declaration slots are allocated only after the
authoritative callable enumeration is available. Both graph scheduling and
compatibility/statistics paths must stop assuming `program.functions.len()`
counts every callable. Native remains sealed direct-call emission only.
Ownership uses the SAME solver with multi-scope membership validation, no
new move lattice or reference algorithm.

## Receiver and argument execution

For a dot call, perform ordinary checking of the receiver exactly once, then
select using its checked semantic type. Check remaining arguments once in source
order after canonical signature instantiation. Evidence collection or lookup
must not evaluate, duplicate, move or borrow the receiver speculatively.

Native execution evaluates receiver once before explicit arguments. Lower the
receiver as the owning first argument through the existing call machinery,
retaining existing left-to-right snapshot rules, move checking and completion
separation. A non-Copy nominal receiver is consumed. `mut self` permits mutation
of the callee's local binding, not new caller-side reference authority.

Qualified calls to receiver methods must use the same selected declaration and
signature, with an explicit first argument. Associated functions without a
receiver are not silently dot-callable. Field access and method calls remain
syntactically and semantically distinct. No arbitrary function-valued field or
callable-object semantics are introduced.

## Universal and concrete trust

Use the SAME checked IR, lowerer, ownership Paths/Transfer/join and actual native
kernels. Parameter leaves remain opaque Sized/non-Copy without canonical bound
evidence. No new ownership checker, Copy flag or witness specialization.

Every generic associated body must pass universal typing and ownership before
MONO_PLAN. Every demanded concrete associated instance is capture-free typed-IR
substituted, canonicalized, lowered anew and concretely analyzed. Only the exact
successful closed CFG capability reaches native layout/emission. CHECK requests
no OBJECT. BUILD cannot publish before all universal and concrete gates pass.
Dead metadata and unused associated bodies retain the existing validation gates.
Exact same-instance recursion deduplicates, expanding instantiation fails boundedly.

Resolved inherent calls become existing canonical direct call targets. There is
no backend method lookup, unresolved `Self`, source-text specialization or runtime
parameter layout. Concrete object symbols must derive from full structural
associated identity and ordered arguments: impl-site semantic module, nominal
semantic declaration path, alpha-normalized self pattern, associated item name,
kind/receiver and ordered impl/method actuals. Raw parameter binder IDs are not
persistent identity. Disjoint specialized impls with the same item name must
produce distinct symbols. Never use process-local IDs or source-body discovery
order. Version semantic/cache keys for incompatible encodings.

## Shared resource authority

Retain the same shared PlanBudget across registry construction, symbolic
applications, selection/overlap, environments, substitution, instance planning,
analysis and symbol generation. Existing per-body caps remain additional.

Proposed unchanged bounds: at most32 visible type parameters per callable
environment,4096 combined applications, depth128,1M expanded nodes,32M work and
128MiB retained/peak-accounted metadata. Scope vectors, method candidate tables,
name comparisons, overlap constraints, canonical keys and retained object call
symbols are charged before growth. Shared clones are not fresh ledgers.
Lookup work includes rejected candidates and repeated comparisons. No lock is
held across graph awaits or recursive budget charging.

## Future trait foundation, not implied acceptance

Reserve architectural space for authoritative Trait/Impl/AssociatedFunction
identities, multi-scope environments, resolved predicates and bounded obligation
proofs. A universal `T: Trait` call needs abstract checked method evidence until
concrete substitution uniquely selects a validated impl. Closed native seals must
reject unresolved trait dispatch even in dead metadata. Canonical Copy permission
must eventually route through one fallible facts service used by typing, lowering,
CFG validation and the SAME ownership engine, never a shadowable identifier or
concrete specialization accident. Trait signature conformance, orphan/coherence,
supertraits and receiver borrowing need separate externally verified contracts.

## Frozen external acceptance corpus

The scratch artifact `associated-items-oracle/corpus.json` has SHA256
`53a4ca56370f69e590af9328659254acf6500220c47388029145ca3f047f48fa`.
Exactly32 initial sources were classified without corrections or growth:
18 supported positives,10 invalid Rust and4 legal unsupported boundaries.
All22 stock-valid sources BUILD/CHECK successfully with both artifacts and
actually run with exit0, exact `associated items oracle ok\n`, and empty stderr.
The10 invalid sources reject BUILD/CHECK with no artifacts: move errors E0382,
type error E0308, unconstrained impl E0207, duplicate visible parameter E0403,
duplicate/overlapping items E0592, private method E0624, private field E0616 and
associated/receiver arity misuse E0599/E0061. The final arity fixture contains
one E0599 and two E0061 diagnostics in both phases.

A separate root invocation independently reran all32 stock BUILD/CHECK and
all22 valid runtimes against the frozen source hash. The disjoint-pattern
positive includes repeated `Pair<T,T>` versus concrete `Pair<i32,u8>` as well
as distinct concrete impls with a shared method name. Legal boundaries retain
real trait dispatch, borrowed receiver adjustments, omitted method generic
arguments and partial type holes. They are stock-valid, not invalid Rust.
These are acceptance fixtures, not a claim that native implementation passed.

## Acceptance before promotion

Freeze exact sources before native implementation. Independent stock BUILD,
CHECK, diagnostics and actual runtimes classify positives, invalid Rust and legal
unsupported boundaries separately. Include by-value move effects, receiver/arg
order, impl/method binder shadowing and forwarding, generic/specialized matching,
all-unused invalid bodies, distinct associated identities, privacy and recursion.
Do not grow or rewrite fixtures to hide a failure.

Focused owned tests, unchanged historical corpus, full hosted check/test/strict
all-target lint/format, actual native example and independent cumulative campaign
remain gates. Export only selected path-redacted runs. Commit/push implementation
and raw campaign receipts separately. Do not reinstall experimental justrust or
claim performance without the pinned comprehensive before/after workload.

## Implemented and hosted validation, October 16, 2026

The native inherent associated slice is implemented with the common callable
registry and ordered `GenericEnv`, not rewritten free-function names. Every
combined-env generic body is universally analyzed before concrete planning.
Canonical self-pattern matching and separately metered overlap unification,
impl-site privacy and simultaneous capture-free substitution feed unchanged
sealed native calls. Shared work/metadata accounting covers rejected candidates,
recursive equality, inherited names and instance-key Hash/Eq work.

The frozen32 CLI fixtures and4 graph tests pass unchanged. Final hosted
`justrust test --no-fail-fast` passes1318 tests, with `cargo fmt --check`,
`justrust check` and strict all-target Clippy also passing. Selected redacted
profiles are in `data/runs/prototype-bootstrap/2026-10-associated-items.jsonl`:
check `20261016-060407167-570915`, test `20261016-060408148-571094` and
lint `20261016-060410698-570906`. Independent actual-source soundness/resource
reviews cleared the corrected patch. No historical source was promoted.

The1547-byte actual example executes successfully with8 workers and exact
`associated items compiled correctly` output. Its complete trace has64 nodes,
134 edges,45 CFG blocks,43 CFG edges and137 locals, with zero failures/drops.
Initial gates caught Self parsing and canonical enum-pattern identity, a missing
injected-self reference-contract offset, normal-stack parser-frame regressions
and unmetered lookup/equality/name allocation. These were fixed in production
without weakening frozen sources, nesting limits or expected outcomes.

Independent cumulative candidate validation is a separate subsequent gate.
This is bounded inherent value dispatch, not traits, borrowed receiver lookup,
full Rust/project compatibility or evidence of a performance improvement.

## Disjoint inherent candidates (2026-10-20)

Body-local inference now disambiguates same-named inherent methods using the
receiver's known ordered type arguments and each candidate's exact impl binder.
It reuses canonical impl-pattern matching without selecting from an expected
result. Unique candidates retain their old path; unresolved or closure-bearing
arguments do not acquire a guessed candidate. Ordinary method checking still
proves applicability and authorizes the final call.

The unchanged backend receiver-order/symbol test now passes, and the complete
109-test backend suite passes (`20261020-041052020-3860517`). Added differential
controls cover concrete disjoint heads, rigid generic tuple arguments, native
receiver/argument evaluation order, and wrong/unknown receiver rejection at
workers 1/8 (all three tests pass in `20261020-041111206-3861165`). Graph integrity
passes all 14 unchanged checks (`20261020-041107061-3861003`), and strict focused
Clippy passes (`20261020-041151787-3862661`). This repairs the narrow inference
regression; full Rust and broader method-inference completeness remain open.

## Independent28-corpus candidate verification

Tracked campaign `compiler-associated-items-20261016-1306` validates exact
revision `16fadabebcc27b309d935eabebdec9755ebd437e` on the metered96-worker
M8a host. Independent parsing verifies all988 cases:595 accepted parity,
35 declared runtime101,276 invalid rejected,81 legal unsupported and1 separate
resource rejection. It verifies934 ordinary workers1/8 runtime comparisons,
70 declared-trap worker runs and128 generated compile/runtime exit checks.
All raw case summaries, source hashes and successful graph captures agree.

The28 corpora and actual executed96-worker example are retained byte-for-byte
or as explicitly observed example data in29 files under
`experiments/results/20261016-m8a-associated-items-*`. All case receipts identify
candidate SHA256 `ab5a6813bd200ba01d9a235e2e84dbc370605d6567083ff52e8b4c323443420c`.
The example source SHA256 is
`2e00a7b39216ad51e5f31a8453cf444c02ce3844264d96392a1f35c0d2629c64`,
with actual exit0 and exact associated-items success stdout. Lifecycle accounting
reports exit0/done/lifecycle_ok, with no billing or tenant bypass. Observed
34,333,747ns total and6,393,580ns engine time are contended functional evidence,
not a speedup measurement. Full Rust and existing-project acceptance remain open.
