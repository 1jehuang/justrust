# Rust compatibility implementation plan

Goal: an independent implementation of the entire Rust language and the
source-to-test-result workflow for existing Rust projects. The first executable
subset is a bootstrap milestone, not the intended final language or architecture.
Normal Rust tools may build our implementation and act as external test oracles.
They must not compile user programs on our native implementation path.

## Integer foundation through 64 bits

`INTEGER-TYPES.md` defines all ten signed/unsigned primitives through 64 bits,
canonical typed low-bit constants, radix literals, signed minima, typed patterns
and width-aware checked native operations. The private eight-byte leaf ABI is
not Rust platform layout or FFI support. `INTEGER-CASTS.md` adds all 100
integer-to-integer conversions with source-signed extension and destination
truncation/normalization, nontrapping casts and private soft source expectations.
Eager integer/Boolean `&`, `|`, `^` preserve exact types and left-to-right evaluation.
Checked heterogeneous integer shifts preserve LHS type and full-count checks;
known invalid counts are diagnosed during native CHECK/BUILD, including two
cases stock metadata accepts before its codegen rejects arithmetic_overflow.
Bounded static diagnostics do not claim general consteval/lint equivalence.
Builtin statement compound assignments preserve RHS-first/current-place order,
initialized scalar reads and single-address dynamic updates. Trait-overloaded
assignments and general compound expressions remain open. Noninteger casts,
128-bit integers, floats and broader inference remain separate work. The full
Rust compatibility goal remains open.

## Builtin reference field slice

`REFERENCE-FIELDS.md` normalizes builtin reference field access to existing real
Deref/Field paths. Privacy, initialized storage, loans, unique mutable authority
and last-use lifetime checks remain the external correctness contract. This is
not trait-based autoderef, method resolution, bare reference indexing, computed
Call-place writes or broader nested-reference support.

## Lifetime elision normalization slice

`LIFETIME-ELISION.md` extends the bounded free-function reference-result contracts
without changing canonical provenance, graph operations or native pointer ABI.
Omitted inputs receive fresh rigid regions in original parameter order. Omitted
outputs require exactly one input reference lifetime position, not one distinct
ID: repeated explicit names in multiple input positions remain ambiguous. An
external stock E0106 oracle corrected the initial design before release. Explicit
outputs never infer missing anonymous-input bounds, and erased NoReference call
contracts remain unchanged. Builtin reference fields and mixed-capability result
inputs are separate delivered slices. Method elision, `'_`, trait autoderef and
general Rust lifetimes remain open.

## Contract and completion criteria

- Define supported editions, targets and stable language versions explicitly.
  Initial compatibility track is edition 2024 on Linux x86-64. Earlier editions
  and other targets need their own tested behavior, not silent reinterpretation.
- Valid supported programs produce correct observable behavior. Invalid programs
  must be rejected even when the erroneous item is unused. Unsupported constructs
  receive an explicit diagnostic, never a successful approximate translation.
- Correct ownership, lifetimes, moves, destruction, unwinding and unsafe rules are
  part of the language contract, not optional optimization details.
- Project compatibility also needs manifests, dependency/version resolution,
  cfg/features, build scripts, proc macros, native linking and test discovery.
  These are graph operations, not a restored Cargo-wide execution barrier.
- Standard-library and dependency compatibility require the relevant language,
  ABI and runtime support. Compiling a toy program or this compiler is a useful
  checkpoint, not proof of full Rust compatibility.
- Pin oracle toolchain and source revisions. Track accepted valid cases,
  correctly rejected invalid cases, known unsupported cases, wrong-code failures
  and diagnostic quality separately. Never count rejection of a valid unsupported
  program as conformance success.

The Rust Reference is a semantic input, not an internal architecture mandate.
Conformance tests, executable differential tests and real projects together form
our evidence. Neither implementation resemblance nor timing alone proves parity.

## Dependency-driven milestones

The ordering below is a starting dependency plan, not a claim that language
features can be implemented in one linear pass. Revisit representations when
new semantics demand it. Each slice must execute end to end and retain graph
accounting, failure diagnostics and benchmark receipts.

1. **Names and source organization.** Module trees, item identity, path resolution,
   imports, visibility, source maps and file loading. First slice: inline modules,
   qualified calls and simple imports. Next slices: external module files,
   grouped/glob imports, re-exports, restricted visibility and edition rules.
2. **Typed data and control flow.** General integer/float types and literals,
   tuples, arrays/slices, structs/enums, construction and field access, patterns,
   match, loops/break/continue and expression/control-flow typing. Introduce
   explicit typed IR and layout facts instead of making the backend guess types.
3. **Ownership and memory.** Places, moves, initialization, references, loans,
   lifetimes, region constraints and drop elaboration. Addressability and alias
   rules must be designed with the IR before broad aggregate/reference support.
4. **Abstraction.** impls/methods, generics, trait solving, associated items,
   coercions, inference, closures and monomorphization. Cache identities include
   substitutions and semantic environment. Handle recursive solving as an
   explicit fixed-point/SCC problem rather than a scheduler deadlock.
5. **Compile-time execution and expansion.** Const evaluation, statics,
   declarative macro hygiene, attributes/cfg and proc-macro execution. Expansion
   and name resolution form a dynamic graph with explicit newly discovered work.
6. **Runtime and platform.** ABI-correct calls/layout, FFI, allocation,
   thread-local storage, atomics, unwinding, compiler intrinsics, target features,
   debug information, linker/object formats and platform support. Existing native
   emitter is a seed, not a promise that its private ABI suffices for Rust.
7. **Library and project integration.** core/alloc/std, dependencies, manifests,
   build scripts, proc macros, native libraries, test/bench harnesses and existing
   project workflows. These overlap earlier phases as soon as prerequisites exist.
8. **Broad conformance and optimization.** Expand real-project acceptance,
   differential/fuzz/minimized regressions and soundness tests. Measure debug,
   release, cold and edit loops. Self-hosting is one gate, not the final gate.

Full Rust is a substantial compiler program, not a feature that this document
claims is already solved. No completion date or full compatibility claim follows
from these milestones.

## Current slice: nominal aggregate ownership

Nonempty braced named structs now use snapshot-scoped nominal identities and
immutable declaration handles. Source values remain non-Copy unless their type
is a supported scalar or a tuple whose every component is Copy. Local-rooted
field consumption moves only the projected non-Copy subtree. The cached CFG
analysis gates both checking and native emission, including whole/partial moves,
reinitialization and control-flow joins. Field privacy includes the accessibility
of inferred and intermediate projected types, not only field names.

The private aggregate ABI is not a platform representation contract. Empty/unit/
tuple structs, struct update/shorthand, derives, general reference regions,
escaping loans, destructors and general Rust data/abstraction features remain
open. Compact shared declaration DAGs and diagnostics have explicit resource
bounds. This slice does not complete ownership or full Rust compatibility.

## Current follow-on: local references

`BORROWING.md` defines the implemented B1 nonescaping local-reference boundary.
Shared references copy, mutable reference handles move, explicit reborrows retain
issuance-time parent authority, and carrier last use determines loan activity.
Native pointers do not substitute for provenance or lifetime verification.
Standalone reference locals can address initialized owned storage and perform
Copy reads or authorized replacements. `REFERENCE-CALLS.md` extends this with
noescape reference parameters, formal canonical referents and expected-site
carrier reborrows. Calls preserve argument authority through invocation and kill
mutable referent selection proofs on return. `REFERENCE-RETURNS.md` adds shared
whole-parameter reference results under explicit named lifetime contracts, with
separate target authority and lifetime dependencies and rigid universal body
proofs. `MUTABLE-REFERENCE-RETURNS.md` adds whole mutable-parameter results as
consumed unique handles, with kind-correct returned origins and parent-loan
freeze/resume. `MIXED-REFERENCE-RETURNS.md` permits mixed inputs: shared results
may derive from either kind, mutable results only from mutable inputs, while
unrelated inputs retain their own capabilities and lifetime constraints.
`PROJECTED-REFERENCE-RETURNS.md` extends results to complete typed
static Field/ConstantIndex input paths, including internal private-field paths
without changing source privacy. Candidate suffixes remain correlated with their
original argument authority, while lifetime dependencies are checked separately.
Enum payload paths, dynamic borrowed indices, trait-based autoderef,
aggregate reference fields, temporary lifetime extension and
general Rust regions remain later work.
Loans born within an SCC and carried over a backedge receive an explicit
unsupported diagnostic. Current acceptance does not establish full Rust
borrow-checker compatibility.

## Current follow-on: irrefutable let patterns

`PATTERNS.md` defines tuple/struct let destructuring, wildcard and per-binding
mutability. Lowering consumes only selected projected places and preserves RHS
effects once. This remains distinct from match/exhaustiveness, refutable patterns,
borrowing, destructors and full inference. Historical legal destructuring boundary
fixtures are promoted to accepted cases only with native execution evidence.

## Current follow-on: fieldless enums and match

`ENUMS.md` defines nominal unit-variant enums, finite enum/bool/unit exhaustiveness,
i32 literal matches with a catchall, wildcard and whole-binding arm semantics.
Enums remain non-Copy. Runtime selection and static ownership reachability are
separate: dead arms remain checked, while wildcard-only and singleton patterns
avoid unnecessary discriminant reads. Payload variants and sum-aware projections
are the subsequent slice below. Guards, or/range patterns, general nested
refutable patterns, derives, explicit
discriminants and empty enums remain open. This does not complete Rust enums,
match, ownership or the full language.

## Current follow-on: fixed arrays

`ARRAYS.md` records bounded fixed arrays and decimal literal indexing, including
once-only repeat evaluation even at length zero, contextual empty literals,
non-Copy indexed move rejection and distinct element selection proofs. Ignoring
an indexed element still inspects initialized array storage. Every array uses
an owning indirect value ABI, including empty arrays. `USIZE.md` extends this
with genuine unsigned 64-bit scalars and runtime bounds-checked indexing. Ordered
CFG operations preserve base/index/RHS effects and unknown-index writes invalidate
contained enum proofs without guessing a selected child. Chained dynamic bases,
slices, array patterns, general constants and broader integer inference remain
open, as do the full compatibility completion gates above.

## Current follow-on: payload enums

`PAYLOAD-ENUMS.md` defines unit/tuple/braced variants, typed downcast projections,
selected field extraction and the tagged private aggregate ABI. Inactive variant
initialization is logical only: a separate must-selected proof authorizes payload
reads. Whole writes/moves invalidate proofs and joins intersect them. Check and
emission retain the same cached analysis gate. Tuple-variant rest, nested
refutable patterns, guards, derives, Drop/references and public layout remain
open. This does not complete Rust sum types or general pattern matching.

## Current follow-on: typed deferred initialization

`DEFERRED.md` exposes first initialization through explicit plain-binding type
annotations. One cached analysis handles uninitialized reads, immutable write
history, branch/match joins and loop backedges. A fresh shadow has a fresh
initialization state immediately. Wildcard-only enum matching remains no-read.
Direct semantic check helpers lower and analyze once, while graph body typing
remains pure and both roots share analysis 109. Backward inference and deferred
destructuring remain open. This is not completion of borrow checking or inference.

## Completed source slice: inline modules and item resolution

Implement nested `mod name { ... }`, ordinary `pub` on modules/functions,
`crate::`, `self::`, `super::` and relative qualified function paths, plus
module-level simple `use path;` and `use path as alias;` for modules/functions.
Resolve forward declarations and imports, distinguish duplicate declaration
names from legal same-name functions in separate modules, enforce privacy and
local value shadowing, and retain a root-only entry function identity.

For direct string-input library calls, reject external `mod name;` because no
filesystem root is provided. Filesystem compilation now supports that form through
source loading before parsing. Continue to reject grouped/glob imports, `pub use`,
restricted `pub(...)`, block-local imports/items and unsupported namespaces.
Existing expression/type restrictions remain in COMPILER.md. Source loading is
bounded and observable but currently expands into one coarse source snapshot;
original per-file semantic/syntax spans remain a following source-map slice.

### Graph representation, interim and intended

- Parse publishes an immutable unresolved module tree and declarations.
- Current slice resolves that snapshot once in an explicit name-resolution node,
  then publishes a canonical flat function arena before signature collection. Body
  checks and code emission consume canonical item identities and remain separate
  per-function operations. Function text order does not imply call dependencies.
- Keep the original unresolved snapshot separate from the resolved arena so no
  reader observes a partially renamed mutable program.
- Whole-source parse, resolution and signature collection are **known coarse
  prototype nodes**. This slice does not solve maximal graph granularity. Future
  per-file/module declaration and per-path lookup facts should expose actual
  prerequisites and make resolution retries/dependencies observable.
- Canonical strings are temporary item identities, not stable cross-build cache
  keys. Persistent identities need snapshot, item provenance, semantic environment
  and compiler-schema version, with full-key equality rather than hash trust.
- Check and build share resolution and body checks. Check never demands machine
  code, and executable publication still waits for all required checks.

### Acceptance

Execute legal multi-module calls, aliases, nested paths, forward resolution,
recursion and legal private-item use. Reject missing/duplicate names, invalid
privacy crossings, alias cycles, unknown imports even when unused and invalid
entry points. Run at one and multiple workers and retain failure graphs.
Compare focused legal/illegal cases with pinned stock Rust outside the compiler
implementation. Require the existing native and scheduler regression suites to
pass. Record actual source sizes, module/function counts and compile latencies,
but make no general Rust speed claim from these small fixtures.
