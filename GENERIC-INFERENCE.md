# First-order generic argument inference

Status: frozen for the bounded implementation below. The explicit generic
foundation in GENERICS.md remains the trust and native identity contract.
This is an incremental language milestone, not full Rust inference.

## Scope

Infer a fully omitted ordered type-argument list for generic free-function calls
and existing struct/enum constructors. Fully explicit arguments retain their
current semantics. Source signatures and type annotations still require explicit
nominal arguments. No partial `_` arguments, trait bounds, methods, default type
parameters, reference inference, coercion/subtyping, local future-use inference,
associated types or global/backward dataflow are introduced.

The first solver has variables only for the current declaration's binder. An
unresolved nested omitted application is opaque evidence, not recursively
specialized or checked speculatively. It may still typecheck normally once
context or other evidence resolves the enclosing application's arguments.
Already explicit nested applications and ordinary known-result calls can supply
known result types. More general connected cross-site inference is future work.

All generic bodies, including unused bodies, still undergo universal checking
with rigid caller parameters and the same ownership solver. Inferred concrete
calls use the identical canonical CallTarget, nominal application interner,
checked-IR substitution, fresh concrete lowering and sealed native plan.
No IR, CFG, native ABI or ownership algorithm change is planned.

## Sequence and evidence

Current calls instantiate a signature before checking arguments. Current nominal
constructors similarly instantiate their owner before checking fields. Insert a
private bounded constraint-evidence pass before this instantiation, not a second
expression checker or a source rewrite.

The evidence pass must not mutate source-checker locals, scopes, reachability,
loans, expression arenas or completion state. It emits no checked expression and
executes no source effect. After solving all ordered arguments, instantiate the
canonical signature/nominal through PlanCtx, then run the ordinary expression
checker once in the existing source order. That ordinary pass remains mandatory
for every child, including opaque evidence, dead code, literal diagnostics,
privacy, constructor field completeness/duplicates, mutability and ownership.
No speculative check/rollback, magnitude-based inference or post-check retagging.

Initial closed evidence grammar:

- Existing locals with declared/resolved types and bounded type-only projections.
  A type lookup is not a read or an initialization/loan proof.
- Bool/unit literals and explicitly suffixed integer literals.
- Unsuffixed integer literals, including the existing grouped literal and unary
  negation/not forms, as deferred integer-domain constraints, not eager i32.
- Tuple expressions and array expressions/repeats with exact existing lengths.
  Empty arrays provide a length but no inferred element type by themselves.
- Known ordinary call results and fully explicit generic call/constructor
  results, obtained from declaration signatures, never a callee body.
- Cast result types as concrete result evidence. Cast operand typing retains the
  existing soft-hint policy and is not evaluated by this evidence pass.
- An available HARD expected result type. CastableToInt is not a hard result
  equality and must not become a generic type-argument constraint.

Other evidence is opaque. In particular arbitrary arithmetic, effectful/scoped
blocks, branches, matches, loops and unresolved nested omitted applications are
not assigned a guessed type from scalar_hint or a tail inspected outside its
scope. They still undergo ordinary checking after complete instantiation when
other constraints suffice. A documented unsupported-inference diagnostic is
required when this bounded solver lacks evidence, not a fabricated Rust error.

Only a known semantic Ty::Never contributes no binding equation. A known ordinary
type with Completion::Never still contributes its ordinary type. Unknown evidence
is not guessed Never. Final ordinary checking preserves completion independently
of type as before.

## Private solver

Use a private type-term algebra, not public Ty holes:

- Slot, identified by the current callee's owner-qualified ParamId.
- Rigid existing Ty, including caller-owned Ty::Param.
- Tuple of terms.
- Array of element term and exact literal length.
- Nominal of source declaration identity and ordered argument terms.

Evidence can retain opaque and deferred-integer leaf markers inside product
shapes. These are not extra generic binder slots or public Ty variables. Opaque
leaves contribute no guessed type equation and taint unresolved components;
deferred integer leaves retain numeric-domain obligations until known structure
or the final permitted fallback resolves them. No partial product with such
markers may escape into a canonical type or application.

Schema substitution introduces slots only while translating the current
declaration's SCHEMA positions. Actual and expected evidence stays rigid even
when recursive caller and callee have the same owner-qualified ParamId. Thus a
recursive `f<T>(x:T)` calling `f(x)` infers the caller's rigid T rather than an
unresolved Slot(T)=Slot(T). Caller parameters are rigid even if they have the
same index or spelling.
Nominal equations compare source declaration plus ordered argument terms, not
field shape, diagnostic names or current symbolic NominalAppId. Tuples require
exact arity, arrays exact lengths and all other rigid types exact identity.
There is no numeric promotion, structural nominal equivalence or hidden reference
permission. Occurs checks and cycle/depth/work bounds precede recursive expansion.

Collect all available equalities and integer-domain obligations before fallback.
Unify deterministically without repeated speculative elaboration. Only unresolved
classes constrained exclusively by unsuffixed integer occurrences default to i32.
A later known u8 argument or hard result u8 must constrain earlier unsuffixed
literals before fallback. Literal range/signed-min/unary legality is then checked
by the ordinary checker under the resolved formal type. Never infer by magnitude.
Noninteger bindings conflict with integer-domain obligations. Opaque evidence
that could constrain a slot taints that slot's unresolved component: do not default
it to i32 merely because another argument is an unsuffixed integer. For example
`same(1, { let x=2u8; x })` requires other known context or an explicit inference
boundary, not an invented i32 type for the opaque block. Other unresolved slots
remain unresolved and produce an honest inference boundary diagnostic.

Do not let scalar_hint commit integer fallback before parent expected context or
peer constraints are available. Any hint-only query may return only justified
known information and must not publish an inferred CallTarget. In particular
`id(1) + 2u8` and `1 + id(2u8)` must not accidentally freeze different integer
instantiations merely because a heuristic runs first. Exact supported peer-hint
behavior must be owner classified before promotion, not guessed from this text.

Use a strict NoFallback hint mode: `scalar_hint(id(1))` returns no hint, while
`scalar_hint(id(2u8))` may return justified u8, without publishing a target or
caching a fallback/expected-sensitive solution. Full resolution receives the
actual hard/peer context and may then apply the bounded fallback policy.
Use fallible hint queries, `Result<Option<Ty>, String>`, so resource, authority,
invalid-declaration and confirmed constraint-conflict errors cannot be swallowed
as absent evidence. Only genuinely unresolved/opaque evidence yields no hint.
On completion validate the entire ordered argument list against the authoritative
caller FactsCtx, existing reference-free restriction, arity and canonical nominal
service. No inference variables may escape into Ty, checked metadata, universal
proofs, graph keys, instance planning or native emission.

## Resource and API ownership

Frontend owns the private solver and evidence walker. PlanCtx may expose bounded
readonly declaration-schema access sufficient for constructor field constraints.
Readonly constructor schema access may be
`PlanCtx::nominal_schema(decl, &PlanBudget) -> (Arc<ParamEnv>, Ty)`, returning the
already canonical symbolic application published by declaration preparation,
with checked authority and work/storage charges. Do not clone source schemas or
intern unresolved inference variables through this API.
No new shared IR operation, public inference variable or separate ownership engine
is needed. Existing explicit paths and declaration resolution remain authoritative.

Use the SAME PlanBudget authority as generic schema construction and native
planning. Preserve 32 parameters per binder, depth128, 1M expanded nodes, 32M
shared work, 128MiB retained/scratch metadata and 4096 combined applications.
Charge evidence visits, terms, constraints, substitutions, equality/occurs checks,
hash/memo capacity and fallback traversal. Reserve all vectors/maps/queues and
peak copies before allocation. Do not reset the ledger per argument, attempt,
scalar_hint query or call. Memoize/reuse bounded pure evidence where repeated hint
queries would otherwise rescan a source subtree. Memo keys must preserve lexical
binding/scope identity, not identifier spelling, and must never reuse a fallback
or expected-sensitive solution under a different context/mode. Prefer caching
pure evidence rather than inferred targets. No source/plan/budget lock may be
held across an await. Existing body/path/frame caps remain additional gates.

## Evidence before GO

Require actual owner and independent read-only feasibility review, then a bounded
frozen stock-classified corpus before implementation authorization. Corpus must
include call and constructor inference from rigid arguments/hard results, deferred
integer fallback and later width constraints, caller parameter rigidity, explicit
argument compatibility, non-Copy moves/partial moves, nominal application identity,
source-order effects, ambiguous/conflicting constraints and preserved universal
unused-body rejection. Legal excluded inference forms must not be labeled invalid
Rust. Stock BUILD/CHECK/runtime statuses and diagnostic codes are individually
observed. No speculative trait/reference support is implied.

After implementation require the unchanged historical suite, strict all-target
lint/formatting, focused solver/resource tests, all-body/no-OBJECT graph gates,
actual native runtime and exact-revision independent full-corpus receipts. A
source-level budget audit is not measured peak-memory proof. No performance or
full Rust compatibility claim follows from this milestone alone.

## Frozen stock corpus

Exactly 28 full sources are frozen with artifact SHA256
`fdd1a268685f4eba121b10d7e354b93ef7375fb7b0fa06639702e0a49c6df845`:
16 supported positives, eight observed invalid sources and four stock-valid
excluded inference forms. All 20 stock-valid sources BUILD/CHECK successfully and
execute with exit0, exact `inference oracle ok\n` stdout and empty stderr. The
eight invalid sources reject in both phases with observed E0308, E0382, E0282 or
`overflowing_literals`. No resource phase exception is introduced.

The source inventory covers later-width constraints, hard result context,
unconstrained literal sites, product shapes/empty arrays, known calls/casts,
struct and enum constructors, rigid forwarding/self recursion, non-Copy moves,
partial moves, argument effects once in order, explicit compatibility, opaque
nested/effect operands with sufficient context and symmetric binary peer hints.
Excluded cases are partial type holes, local future-use inference, unresolved
nested omission and opaque numeric evidence without sufficient context. No
source bytes or expected classifications may grow/change without a separately
observed correction. Stock runtime at the unconstrained local site alone is not
proof of its inferred fallback type, which also needs a checked-type regression.
The cast contrast `id(256) as u8` executes as0, consistent with primary call
expectation guidance using only hard expected types, not CastableToType.

## Delivered acceptance checkpoint

The frontend-only implementation passes all 1,270 hosted tests, including the
unchanged exact28 corpus, four graph groups and three focused frontend groups.
Both independent actual-source reviewers cleared the corrected implementation.
The existing deep-parser and 120-cast resource gates remain unchanged. Final
check/test/strict all-target lint runs are 20261016-052541976-266990,
20261016-052542583-267149 and 20261016-052543995-266808, retained in
`data/runs/prototype-bootstrap/2026-10-generic-inference.jsonl`.

The actual eight-worker native example executes with exact expected stdout. One
historical generic-enum match fixture is promoted after unchanged-source stock
and native workers1/8 runtime parity, not rewritten to fit the implementation.
The separately tracked independent campaign on
`6cef2b5c6feb16ea30692d94b0edb759fd2d91d2` verifies all 956 cases in 27
corpora, 898 ordinary and 70 declared-trap workers1/8 runtime checks plus 128
generated compile/runtime exit checks. The actual 96-worker example executes
with complete capture. Raw receipts are retained under
`experiments/results/20261016-m8a-generic-inference-*`.
No full Rust completion or compile-speed improvement is claimed.
