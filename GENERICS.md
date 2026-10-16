# Universal generic templates and native instances

Status: frozen reviewed contract, with exact external sources classified below.
Implementation still requires a separate explicit coordinator GO. No native generic
support or performance claim is made. Full native Rust remains the goal. This foundation is not
traits, core/std compatibility, or full Rust completion. User-source compilation
must never delegate to another compiler, interpreter, assembler or linker.

## First external scope

Support implicitly Sized type parameters on free functions, braced structs, and
unit/tuple/braced enum variants, with explicit ordered type arguments. Admit
`id::<u64>(value)`, generic-to-generic `id::<T>(value)`, nominal type applications,
`Wrap::<T> { value }`, and `Maybe::<T>::Some(value)`. Paths and arguments are
resolved structured data, not angle-bracket text appended to names. Nested type
arguments require type-context splitting of existing `>>`, `>>=` and `>=` tokens
without changing expression shift, compound-assignment or cast-ambiguity guards.
Omitted arguments and `_` inference holes remain explicitly unsupported.

Ordinary argument evaluation, moves, field privacy, aggregate transport, payload
matching and existing control flow retain their existing semantics. Every type
argument must be reference-free recursively, including nested nominal fields,
tuples and zero-length arrays. Generic declarations/bodies cannot introduce
references in this first scope. Existing nongeneric reference support must not
regress. Excluded interactions produce explicit unsupported diagnostics, not
claims that otherwise valid Rust is invalid.

No bounds, Copy marker special cases, derives, traits, methods, associated types,
const generics, default type arguments, higher-ranked binders, `?Sized`, generic
function values or omitted generic-argument inference in this foundation.
Unconstrained T is universally non-Copy. Nominal structs/enums remain non-Copy
without a separately implemented Rust capability. Primitive-name shadowing in a
type-parameter scope must resolve as a parameter where Rust permits it, not be
prematurely parsed as a concrete primitive.

## One semantic operation graph, two checked phases

Use a common checked operation representation and immutable semantic type facts,
not a second permissive checker and not substitution of a witness integer type.
A rigid parameter has owner-qualified binder identity and parameter index. It
is Sized, has no fields or variant structure, and has no native size or ABI.
Type equality compares binder identity, never diagnostic spelling.

Use the existing common semantic Ty with a real `Ty::Param(ParamId)` case.
`ParamId { owner: BinderId, index: u16 }` has a unique declaration binder across
functions, structs and enums in a snapshot. `ParamEnv` records the allowed binder
and arity and must come from the authoritative declaration environment, not a
caller-forged body annotation. `FactsCtx::Universal(ParamEnv)` and
`FactsCtx::Concrete` centralize
bounded type validation, Copy facts, projection typing and move shape. An interned
TypeId redesign is not required for this first foundation.

The bounded borrowed shape view is Scalar, Product, Array, Sum, OpaqueParam or
Never. It exposes existing fields/variants without constructing uncharged expanded
vectors. Parameter shape has no physical size. No wildcard unknown-type fallback
or default eight-byte layout is permitted. Applying a nominal schema substitutes
ONLY that schema's binder; remaining parameters must belong to the caller's
validated environment. Function-call substitution follows the same capture-free
rule, never parameter-name or index-only replacement.

A template body carries a validated parameter environment. Distinguish structural
validation from successful analysis: a private `TemplateCfg` is validated under
an authoritative ParamEnv, and `VerifiedTemplateCfg` binds that exact immutable
CFG to its successful universal analysis AND the exact immutable checked template
from which it was lowered. An instantiator cannot combine proof A with source B.
A `ClosedCfg` proves exhaustive
zero-parameter/canonical metadata validation before concrete analysis.
`ConcreteCfg` then binds that exact immutable closed CFG to its successful
concrete analysis, never a caller Boolean or analysis of a different body.
Native emission and layout require `ConcreteCfg`, not merely `ClosedCfg`.
No parameter may
reach physical size, field offsets, frame allocation, calling convention or
object emission. The normal concrete CFG/ownership validation still runs after
instantiation, independently of universal template analysis.

## Universal checking, including unused declarations

Check every declaration signature, generic nominal schema and function body,
including unused generic functions, before CHECK or BUILD can succeed. Checking
only demanded monomorphizations is unsound as an implementation of Rust generics.
`fn bad<T>(x:T)->(T,T){(x,x)}` must fail even with no callers or only integer callers.
Arithmetic, comparisons, casts, indexing and field access cannot assume that an
opaque T is an integer or a particular aggregate. Calls use resolved declaration
signatures and symbolic arguments, never capabilities inferred from a callee's
observed specializations.

Reuse the SAME ownership move-path construction, initialization transfer,
join/intersection and worklist machinery through a bounded type-facts/move-shape
adapter. Under concrete facts, existing reference carrier/external-referent
paths and borrow analysis remain intact. The generic no-reference fence must
not disable nongeneric borrow checks. An opaque Sized parameter contributes one indivisible non-Copy path.
Known tuple/array/nominal shape exposes its structural children, including opaque
parameter leaves. Distinct field moves, parent readiness, enum proof selection,
loops and branch joins obey the existing transfer rules. A second move of T
fails universally. Copy/Move tags must be validated against universal type facts,
not blindly trusted because a caller constructed an Operand.

No fake Unit/struct witnesses, separate relaxed move checker, body-erasing
NoReference shortcut for hidden references, or arbitrary specialization-based
acceptance. The first-scope reference exclusion must be checked before universal
analysis and again for all concrete type arguments, without erasing existing
nongeneric reference contracts.

## Nominal identity and typed substitution

Separate source declaration identity from nominal application identity.
`Wrap<i32>` and `Wrap<u8>` are different semantic types even with one source DefId.
Intern applications by declaration plus ordered structural type arguments, both
symbolic and concrete. Keep source declaration/module identity for privacy and
diagnostics. Variant/downcast owner identity is the nominal application, not
only the base enum declaration. Wrong-owner proofs must continue to fail closed.

Instantiate CHECKED typed IR capture-free, not source text or reparsed ASTs.
Substitution must cover local/expression/signature types, all nominal fields,
constructors, patterns, payload bindings, Place Downcast owners, switch owners,
call signatures and resolved callee type arguments, aggregate/repeat metadata,
and any retained contract/provenance metadata. Re-intern each resulting nominal
application consistently. ExprId and LocalId remain body-local handles.

Lower the resulting concrete checked body anew, choosing Copy/Move from concrete
type facts. Do not relabel old CFG Move tags or mutate an already accepted
concrete body. Revalidate, lower and analyze each concrete instance before native
emission. Symbolic declaration analysis cannot be replaced by these analyses.

## Instance discovery and graph trust gate

Function instance identity is resolved function declaration plus ordered canonical
concrete type arguments and target/ABI/semantic configuration. Snapshot-local
handles are not persistent cache identities. Do not use Arc addresses, process
SnapshotId, discovery order, pretty names or unchecked hash collisions as
semantic identity. Bounded structural encodings or collision-checked interners
are acceptable. Deterministic relocation symbols derive from instance identity,
not source-name substitution. Preserve the exact main entry contract.

Replace source-function-count-only fact slots with separate universal declaration
facts and a bounded sealed concrete instance plan. Seed EVERY nongeneric function
to preserve the existing unused-body gate. Follow typed calls by capture-free
substitution in a deduplicating worklist. Publish the immutable instance frontier
before allocating per-instance BODY/LOWER/ANALYZE/OBJECT slots.

CHECK joins all universal declaration analyses and all planned concrete analyses,
and requests no objects. BUILD joins those same gates before publication and
links only the sealed object frontier. Native compatibility emit helpers must
use this common plan, not introduce a second reachability-only compiler path.
Exact-instance self or mutual recursion reuses an existing key and needs no
callee BODY/OBJECT wait cycle. Growing instances hit a useful explicit resource
boundary, not an infinite queue or invented declaration-invalid diagnostic.

Validate unused nominal declaration well-formedness, binder uniqueness/arity and
Rust's nominal unused-parameter rules, and by-value recursion, then
validate each substituted by-value dependency DAG BEFORE publishing immutable
layout handles. Finite nested applications are not themselves cycles. Type
arguments that transform during recursive expansion cannot bypass the guard.

## Proposed resource contract for owner review

All limits are explicit supported-resource boundaries, not new Rust semantics:

- At most 32 parameters per binder.
- At most 4096 combined function instances and symbolic/concrete nominal
  applications per program, including applications retained during universal
  checking. Do not renew this ledger at the concrete planning phase.
- At most 128 active type/substitution/instantiation depth.
- At most 1,000,000 expanded semantic/operation nodes across a plan.
- Shared planning work at most 32,000,000 charged steps and retained planning,
  type and move-shape metadata, plus charged temporary planning scratch, at most
  128 MiB.
- Bound canonical key/symbol byte construction before string/vector growth.

Charge before interning, queue growth, type expansion, adapter allocation or
solver-state allocation. Budgets are shared over the full closure, not renewed
for every instance. Existing per-function path/frame/borrow/body limits remain.
Cached type DAGs must not bypass expanded path or depth limits. Charge every
shape lookup, child traversal and repeated DAG occurrence; zero-length arrays
still validate their element and Copy facts. Symbolic opaque
paths and concrete expanded paths both receive accounting. Nominal by-value
layout cycles must fail before Arc recursion or layout stack overflow. Exact
function-instance recursion remains permitted and deduplicated as above. Unused
function type parameters are permitted, unlike invalid unused nominal parameters.

## Shared API publication and ownership

The following owner-agreed interfaces are frozen for coordinated publication:

- `ir.rs`: unified `DeclKind`, `DeclId { snapshot, index: u32 }`,
  `BinderId { declaration: DeclId }`, `ParamId`, private authoritative ParamEnv,
  and DISTINCT `NominalAppId { snapshot, index: u32 }`.
- `StructDef` and `EnumDef` identity is NominalAppId, with separate
  `source_decl: DeclId` and `type_args: Arc<[Ty]>`. VariantId.owner is
  NominalAppId. No DefId compatibility alias may conflate the two domains.
- `CallTarget { declaration: DeclId, type_args: Arc<[Ty]> }` is stored in checked
  and CFG calls. A name is diagnostic/relocation transport, not call identity.
  `GenericSignature { param_env: Arc<ParamEnv>, signature: Signature }` is the
  declaration schema; a checked call signature is substituted into caller terms.
- `ir.rs`: PlanLimits, FactsCtx and cloneable PlanBudget with ONE shared authority
  across clones. Fallible work/node/application charges, RAII retained/scratch
  byte reservations and operation-local depth checks receive `&PlanBudget`.
  Cloning the handle must not clone or reset counters. Parallel checks cannot
  race past limits or use a shared global recursion-depth counter.
- `frontend.rs`: authoritative DeclRegistry, structured DeclSemanticPath,
  nominal schemas, NominalInterner and PlanCtx retained through instance
  discovery. `PlanCtx::from_program(program, budget)`, declaration/schema/binder
  lookup, intern_nominal, substitute_ty and instantiate_signature are bounded
  fallible services using that same budget. Mutable service access is
  synchronized where needed; no lock may be held across graph await.
- `ir.rs`: one exhaustive instantiate_checked visitor requires the verified
  exact template capability, typed arguments and budget. Its narrow
  SubstitutionCx trait exposes nominal_application, intern_nominal and
  instantiate_signature; frontend PlanCtx implements the trait. The IR visitor
  performs binder-qualified builtin type substitution itself, with no arbitrary
  rewrite-type callback. Frontend schema construction may reuse the same type
  substitution helper. No duplicate root/source instantiator and no unsealed
  generic input plus a caller promise of a matching proof.
- `ownership.rs`: borrowed MoveShape adapter and universal/concrete wrappers use
  the same existing solver. `cfg.rs` and ownership coordinate the private
  structural/pre-analysis and successful-analysis capabilities above.
- `backend.rs`: only ConcreteCfg enters native layout/emission, including legacy
  compatibility helpers. Explicit Param rejection remains defense in depth.
- `compiler/mod.rs`: separate declaration facts, canonical instance keys/symbols,
  sealed frontier, instance slots and all-body CHECK/BUILD publication gates.

Publish identity/API shapes and migrate exhaustive production matches BEFORE
adding bounded focused regressions. Owners send a compile-ready checkpoint
before focused builds; coordinator runs integrated checks only after shared
publication. No overlapping owner edits, agent commits or nested spawning.

## Exact external oracle checkpoint

The frozen source artifact is
`$JCODE_SCRATCH_DIR/generics-oracle/corpus.json`, SHA256
`b792c18770c85af3754d32ac6298a0a5277a868ce314448e212dc25f411c4d97`.
Its 28 cases are 16 positive, 8 invalid, 3 legal-unsupported and 1 separately
classified resource error. Root independently verified all compile, metadata and
runtime outcomes with stock rustc 1.98.1, edition 2024. This is NOT native generic
acceptance evidence and does not authorize implementation by itself.

The expanding-instance resource case has stock BUILD exit 1 with a recursion
limit diagnostic, metadata CHECK exit 0, and no executable/runtime. Preserve
those individual statuses. Native CHECK and BUILD may both reject earlier with
a useful bounded generic-instantiation resource diagnostic. Do not relabel this
case invalid Rust or legal-unsupported, and never treat timeout, signal, parser
rejection or arbitrary compiler failure as a passing resource guard. No new
historical classification promotions are authorized.

## Required evidence before implementation and promotion

All six owner/architecture reviews are clear. The exact 28-case
source/classification/stdout artifact above is frozen and independently
stock-classified. Shared API ownership is agreed as published above. These are
implementation prerequisites, not evidence that native generics already work.

Require valid identity/wrapper/payload/partial-move/forwarding/exact-recursion and
multiple-specialization cases. Require UNUSED bad duplication/operators/moves,
wrong nominal applications/arity and forged private metadata to fail. Hidden
reference exclusions and expanding recursion require honest boundary diagnoses
and individually observed stock statuses, not guessed legal-unsupported labels.

Internal gates must cover closedness before all native layout entrypoints,
consistent substitution of every nominal owner, binder capture/arity errors,
all-body template gates with no concrete caller, deterministic instance reuse,
no OBJECT during CHECK, no publication after any declaration/instance failure,
resource limits/cycles and current reference/numeric regression suites. Retain
exact raw receipts, redacted run profiles and actual native runtime outcomes.
No speedup claim without the pinned comprehensive before/after benchmark.
