# Full Rust implementation tracker

The requested outcome is full Rust support, not completion of a sequence of toy
subsets. This checklist stays open until compatibility evidence supports closing
it. `RUST-COMPATIBILITY.md` defines the contract and dependency ordering.

## Completed foundations, not full compatibility

- [x] Independent graph runtime with shared immutable facts and lifecycle telemetry.
- [x] Direct native Linux x86-64 executable generation for the initial subset.
- [x] Inline modules, qualified calls, simple imports and namespace/privacy checks.
- [x] External module source trees with bounded loading and observed source facts.
- [x] Scalar checked-function IR shared directly by checking and native emission.
- [x] Unlabelled loop values, break/continue and exact native temporary cleanup.
- [x] Separate semantic Never typing from completion, including typed dead tails.
- [x] Nested Copy tuples, field projections/mutation and by-value aggregate calls.
- [x] Explicit cached CFG facts shared by check and native emission, with provenance.
- [x] Cached storage/initialization/move-path dataflow shared by check and emission.
- [x] Nonempty nominal structs, field privacy and source-level whole/partial moves.
- [x] Irrefutable tuple/struct let patterns with selective moves and wildcards.
- [x] Fieldless nominal enums and scalar/enum match with exhaustive coverage.
- [x] Typed deferred local declarations, flow-sensitive first writes and shadowing.
- [x] Payload enums, typed downcasts, bounded selection proofs and native tagged values.
- [x] Fixed arrays, repeat effects, literal indexing and private by-value array ABI.
- [x] Genuine 64-bit usize and ordered, bounds-checked runtime array operations.
- [x] Nonescaping local references, canonical provenance and last-use loan analysis.
- [x] Noescape reference parameters and genuine expected-site carrier reborrows.
- [x] Shared/mutable static input-path returns with explicit or elided regions.
- [x] Builtin reference field projections and capability-sensitive mixed inputs.
- [x] Reference-free generic templates with universal move checking,
      application-qualified nominal identity and sealed concrete native instances.
- [x] Bounded first-order omitted call/constructor arguments with deferred integer
      evidence, rigid caller parameters and fallible no-fallback peer hints.
- [x] Hosted acceptance, stock differential cases and qualified benchmark receipts.
- [x] Native f32/f64 arithmetic, IEEE comparisons, saturating numeric casts and
      canonical Copy/Clone recipes, with bounded first-order float inference.
- [x] Lexically resolved labeled loops/while/blocks and target-specific cleanup.
- [x] Universally checked source trait default bodies, override-aware native
      dispatch and explicit Sized proof for owned trait Self values.
- [x] Nested grouped imports, named aliases, type-only self leaves and validated
      empty prefixes, with global logical expansion bounds.
- [x] Named public reexports with canonical identity, inherited variant export
      eligibility and namespace-specific original/intermediary visibility.
- [x] Normalized lexical restricted visibility for supported declarations,
      nominal/field/method access and non-widening named reexports (VISIBILITY.md).
- [x] Canonical monomorphic/first-order generic source type aliases with eager
      unused validation, bounded hygienic expansion and fixed/explicit generic
      constructor/pattern arguments (TYPE-ALIASES.md). Inferred alias constructor
      and pattern arguments remain explicitly unsupported.
- [x] Native i128/u128 arithmetic, exact numeric casts and scalar transport under
      INTEGER128.md, without claiming platform ABI compatibility.
- [x] Configured text/file SOURCE preparation, bounded cfg/cfg_attr and inert
      attribute policy, cfg-before-child-IO discovery and original syntax spans
      under SOURCE-PREPARATION.md. Full inactive syntax and expansion remain open.
- [x] Explicit source-locked original attributed identity declaration at runtime,
      retained constness with universally checked forwarding-only bodies and
      scalar-root shared/mutable inference replay under CORE-IDENTITY.md. This
      is not CTFE, a library crate artifact or whole-core compilation.

## Remaining implementation program

The categories below cover the known program, not an exhaustive decomposition of
Rust. Having every category named is not the same as having every semantic rule,
integration edge and acceptance case in executable todos. Remaining unknowns need
discovery through a pinned semantic inventory, library builds, real projects,
independent review and minimized failures. Add newly discovered work here before
claiming the affected gate is complete. Full Rust support is **not complete**.

Delivered contracts, including later extensions, take precedence over historical
first-client exclusions. In particular, bounded associated dispatch, canonical
Copy/Clone, generic reference carriers, source trait defaults, floats, labels and
shared byte slices are foundations, not absent categories and not category-wide
completion. `GENERIC-CARRIERS-ACCEPTANCE.md` distinguishes design freeze from real
native acceptance. Preserve the checkpoint evidence below without treating its
historical test counts as the current total.

### Gate rules and dependency map

Each gate needs a scoped contract, named owner, dependency/API agreement, frozen
positive/invalid/legal-unsupported cases, actual public CHECK/BUILD/run evidence,
and retained regression receipts. Invalid unused declarations and dead metadata
must still reject. CHECK must not demand code generation. Failed correctness,
unsupported input, resource limits and infrastructure faults remain distinct and
must never publish a successful executable. Internal graph tests supplement,
not replace, actual source acceptance. Every supported extension needs review of
its interactions with prior types, ownership, substitution and runtime behavior.

The dependency labels below constrain individual slices, not whole-program phase
barriers. Independent design, oracle classification and harness work starts now.
A downstream native client starts as soon as its exact prerequisite slice passes:

- G0 (contract/discovery) and G10 (conformance) run continuously across all gates.
- G1 (source/names) and G2 (typed data) extend the delivered semantic foundation.
- G3 (ownership) and G4 (abstraction) co-design shared type/region interfaces on
  G1/G2. Neither waits for the entire other category to finish.
- G5 (expansion/const) uses G1/G2 and applicable G3/G4 semantics. Its discovery
  can add declarations and dependencies before other bodies are available.
- G6 (runtime/targets) consumes validated G2/G3/G4 facts. ABI design and external
  harnesses can proceed before those consumers are implemented.
- G7 (libraries) advances incrementally on G1-G6, beginning with pinned core
  blockers rather than waiting to attempt the entire standard library.
- G8 (projects) starts manifest/discovery work now, then integrates G5-G7 for
  native dependency, build-script and proc-macro execution.
- G9 (graph/cache/security) is cross-cutting. Persistent/distributed reuse needs
  explicit identity, validation and isolation gates beyond in-snapshot sharing.
- G11 (performance/completion) measures each accepted slice, but broad promotion
  waits for the declared language, library, target and project gates.

### G0. Versioned compatibility contract and discovery

- [ ] Pin the stable language/toolchain oracle revision, Rust Reference/library
      source revisions, edition matrix, target triples and supported target
      features. Edition 2024/Linux x86-64 is the initial track, not evidence for
      Rust 2015/2018/2021, other platforms or all future stable releases.
- [ ] Build a traceability ledger linking semantic requirements and known
      unsupported fixtures to a contract, implementation owner, dependencies,
      acceptance cases and evidence. Audit lexical grammar, attributes, lints,
      edition differences and cross-feature interactions as well as feature names.
- [ ] Maintain a discovery queue for unspecified/ambiguous behavior, unsupported
      library requirements and new failures. Separate stable-contract obligations
      from unstable features without silently narrowing the full Rust goal.

**Next:** freeze the initial version/edition/target matrix and classify every
current legal-unsupported boundary against it. **Acceptance:** each matrix cell
has an explicit supported/unsupported/not-yet-investigated status and linked
cases. An empty discovery queue or a long checklist is not proof of exhaustiveness.

### G1. Source organization, syntax and edition behavior

- [ ] Preserve original file/expansion spans, source maps and stable declaration
      provenance instead of relying on concatenated source positions. Original
      file syntax spans are delivered under SOURCE-PREPARATION.md; general
      semantic and macro-expansion provenance remain open.
- [ ] Complete glob imports, namespaces, block-local items/imports, cross-crate
      paths and edition-specific resolution. Bounded restricted visibility is
      delivered under VISIBILITY.md with stock/native scope acceptance. Bounded
      grouped use trees are delivered under GROUPED-IMPORTS.md. Named public
      reexports are delivered under REEXPORTS.md with stock/native privacy and
      identity acceptance.
- [ ] Cover remaining lexical/literal/item forms, attributes, cfg removal,
      recursive module discovery and useful multi-file/expanded diagnostics.
      The bounded source-envelope slice supports item/module configuration and
      explicit attribute policy, not arbitrary cfg positions or full Rust syntax
      in disabled items. The latter remains explicitly unsupported when outside
      the admitted syntax-only grammar.

**Next:** extend original provenance into semantic diagnostics and expansion,
then token trees/hygiene and source-owned library roots without weakening the
delivered source-map and import/privacy corpus. **Acceptance:** forward/cyclic/ambiguous lookup and unused invalid
imports have pinned outcomes, real file spans survive loading, and edits or
missing-file changes invalidate only justified consumers without stale lookup.

### G2. Typed values, patterns, inference and control flow

- [x] Native i128/u128 scalar arithmetic, numeric casts, transport and exact
      typed constants under INTEGER128.md (not platform ABI or full Rust).
- [ ] Complete char, str/string literals, raw pointers, function items/pointers,
      remaining type-alias inference/generic forms, never coercions and literals.
      Bounded canonical source aliases are delivered under TYPE-ALIASES.md.
      Extend float/library behavior without confusing private scalar transport
      with platform layout or profile-correct arithmetic behavior.
- [ ] Complete empty/unit/tuple structs, construction/update forms, unions,
      discriminants, recursive types, alignment/layout and repr attributes.
- [ ] Generalize slices/DST metadata, array/slice coercions and indexing beyond
      delivered shared `&[u8]`; retain bounds/evaluation order and size limits.
- [ ] Complete nested/or/range/ref/@/slice patterns, guards, match ergonomics,
      exhaustiveness/usefulness, if-let/let-else forms and destructuring assignment.
- [ ] Extend bidirectional/local inference, coercion sites, partial type holes,
      omitted method arguments and control-flow joins, preserving rigid universal
      parameters and diagnostics for unresolved constraints.

**Next:** enumerate remaining type/pattern forms and freeze one end-to-end slice,
such as general slice metadata or nested patterns, with resource accounting.
**Acceptance:** source execution and invalid typing agree with the pinned oracle,
including effect order, overflow profile, empty values, unreachable bodies and
malformed typed IR. Large scalar arrays need a measured analysis representation
that removes the recorded 44,417-byte Adler2 resource blocker without weakening
move/initialization checks. Adapted Adler2 remains distinct from unchanged-crate
acceptance.

### G3. Ownership, regions, destruction and memory validity

- [ ] Complete reference-containing nominal fields/aggregates and lifetime type
      parameters, variance, subtyping, implied/outlives bounds, higher-ranked
      regions, `'static`, placeholder lifetimes and general region inference.
- [ ] Extend dynamic borrowed places, temporary scopes/lifetime extension,
      aggregate carrier flows, general reborrows and loop/SCC-carried loans.
      Preserve the delivered current-cell and per-occurrence provenance rules.
- [ ] Implement real Drop checking/elaboration, conditional drop flags, partial
      initialization, assignment replacement and exact destruction order across
      returns, branches, loops, breaks, panic/unwind and closure/async captures.
- [ ] Specify unsafe validity, raw-pointer operations, alias/provenance rules,
      interior mutability and UnsafeCell interactions. Track genuinely unresolved
      language-model questions rather than inventing permissive defaults.

**Next:** agree a shared drop-obligation/cleanup IR contract and freeze observable
Drop-order plus rejected-escape fixtures. In parallel classify aggregate-region
and loop-loan boundaries. **Acceptance:** required drops occur exactly once on
supported normal or exceptional paths, invalid escapes and live-alias conflicts
reject even in unused bodies, and CHECK/emission consume the same verified
ownership facts. Explicitly permitted leaks/forgotten values are not compiler
cleanup failures.

### G4. Traits, generics, methods and suspended computations

- [ ] Add trait type/lifetime/const parameters, generic methods, where clauses,
      associated types/constants, projection normalization, GATs, defaults and
      supertrait obligations beyond the bounded delivered source-default slice.
- [ ] Complete coherence/orphan rules across crates, auto traits/negative impls
      where stable, recursive obligation convergence and precise ambiguity.
      Resource exhaustion must not become NotProven or successful dispatch.
- [ ] Implement trait-based autoderef, overloaded operators/indexing, coercions,
      dyn compatibility/vtables, unsizing and `?Sized` with authoritative layout
      and receiver adjustments rather than name- or pointer-based permissions.
- [ ] Complete closures and capture modes, callable traits, function values,
      iterator/for desugaring, async/await/Future state machines and pinning.
      Include capture lifetimes, suspension-safe moves, cancellation and Drop.
- [ ] Extend monomorphization/recursive instance planning to the new forms while
      retaining universal checks, binder-qualified substitution and sealed native
      instances. Optimization must not bypass unused-body validation.

**Next:** freeze trait-argument/associated-type goal and normalization APIs with
ownership/type owners, then use a small Iterator-like source client. Closure
capture design can proceed independently against the agreed place/region API.
**Acceptance:** generic bodies prove obligations without favorable concrete
witnesses, ambiguous/overlapping impls reject, and concrete dispatch, closures,
iteration and suspended-state destruction each have executable oracle cases.

### G5. Expansion, compile-time evaluation and generated work

- [ ] Implement token trees, macro_rules matching/repetition, hygiene and mixed
      expansion/name resolution, including crate identity, spans and recursion.
- [ ] Complete attributes/cfg/cfg_attr, derives and function/attribute/derive
      procedural macros with host/target separation and compatible token/span APIs.
- [ ] Implement const/static/const-fn evaluation, promoted values, const generics,
      target-aware arithmetic/layout, allocation/provenance and diagnostic rules.
      The bounded shift evaluator is not general const evaluation.
- [ ] Make newly expanded/generated declarations explicit graph discoveries;
      track observed files/environment and conservatively disable reuse for
      uncontrolled effects. Enforce execution/resource/isolation policies.

**Next:** freeze token/span/hygiene and target-aware const-value contracts, then
classify a macro_rules and array-length const corpus independently. **Acceptance:**
expansion order/hygiene, const failures and generated diagnostics match pinned
cases, invalid unused expanded items reject, and changed cfg or observed inputs
cannot reuse stale expansions. Proc-macro acceptance executes a real native
host macro and compiles its generated target source.

### G6. Runtime, FFI, linking and target expansion

- [ ] Define target data layouts/calling conventions, object/relocation/symbol
      formats, crate artifacts and linking, repr(C)/transparent/packed/aligned,
      extern calls/statics, native libraries, variadics and stable ABI boundaries.
- [ ] Implement allocation/deallocation, startup/termination, panic abort/unwind,
      cleanup/personality support, TLS, threads, atomics/memory ordering, intrinsics,
      target features and inline/global assembly for the declared target contract.
- [ ] Provide debug/source information and runtime failure/exit diagnostics.
      Expand architectures/OSes only with target-specific executable evidence.

**Next:** freeze the initial Linux x86-64 ABI/layout matrix and bidirectional
native C harness, including aggregates and callbacks. **Acceptance:** platform
harnesses link and execute both directions, layout/ABI match the platform,
unwind/abort and thread/TLS/atomic tests have defined outcomes, and unsupported
targets reject explicitly. The current private aggregate ABI does not pass this
gate, nor does successful direct ELF emission alone.

### G7. Native core, alloc and std bring-up

- [ ] Establish pinned library source manifests, language-item/intrinsic inventory,
      sysroot identity and library feature/target configuration without embedding
      rustc or pretending catalog recipes compile the libraries.
- [ ] Compile and exercise no_std/core first, then alloc with an actual allocator,
      then target std with platform runtime support. Feed every new blocker into
      G0-G6 with a minimized reproducer and dependency owner.
- [ ] Cover canonical library trait semantics, collections, strings, formatting,
      I/O, synchronization, panic handling and library tests in executable clients.

**Next:** attempt an inventoried pinned core discovery/check run and publish the
first unsupported blockers rather than claiming a successful library build.
**Acceptance:** each library stage compiles on the native path, links real clients
and runs the declared tests. No stock-compiled library artifacts may conceal
missing native compilation. Catalog Copy/Clone support is not core acceptance.

### G8. Existing Cargo projects and source-to-test workflows

- [ ] Parse existing manifests/workspaces, targets/profiles and configuration;
      implement lockfile-compatible package/version resolution, registries/git/path
      sources, patches, features/resolver versions and host/target dependency kinds.
- [ ] Integrate multiple crates, generated inputs, build-script directives,
      environment/rerun rules, native tools/libraries and proc-macro compilation
      and execution as graph operations, not a Cargo-wide phase barrier.
- [ ] Support unit/integration/doc tests, test discovery/filtering and exit status,
      examples, benches and requested outputs with compatible diagnostics.
- [ ] Keep explicit stock fallback separate and labeled. Never send private
      formats to stock tools or count fallback as native compatibility.

**Next:** freeze a tiny unchanged workspace containing path dependencies, features,
a build script, a proc macro and tests, plus manifest/resolution fixtures that can
run before native compilation is available. **Acceptance:** lockfile/feature and
host/target selections are correct, unchanged project commands execute native
outputs/tests, failures remain visible, and edits to generated/native inputs
invalidate the necessary graph. Progress from this fixture to pinned independent
real crates and multi-crate applications, not just adapted source snippets.

### G9. Integrated graph, persistence, security and operations

- [ ] Split measured coarse source/resolve/signature work into independently
      publishable facts with complete declaration-discovery prerequisites and
      supported recursive convergence protocols, not hidden phase barriers.
- [ ] Define stable cross-edit identities and versioned full semantic keys,
      dynamic/negative-input validation, fine-grained invalidation and exact
      build/check/test variant reuse verified against clean recomputation.
- [ ] Add bounded asynchronous persistence, atomic manifests, durable checkpoint
      contracts, recovery, eviction and corruption/crash/cancellation tests.
- [ ] Specify authenticated provenance, artifact integrity and authorization for
      lookup/read/publish. Prevent poisoning and private-derived data/metadata
      leaks. Implement multitenant service policy only in the cloud repository.
- [ ] Sandbox build scripts/macros/tools/tests, enforce resources and fairness,
      and measure local/remote placement, transfer, backpressure and retry costs.
- [ ] Retain exact structural accounting, incomplete-capture markers, redaction,
      wait causes and critical-path explanations with measured overhead.

**Next:** freeze an edit/invalidation matrix and cache threat model while compiler
owners expose finer facts. **Acceptance:** worker/cache/schedule variations agree
with clean results, untracked inputs prevent unsafe reuse, crashes never publish
partial success, adversarial cross-tenant/forged-artifact tests fail closed, and
shared physical work is counted once. In-memory snapshot memoization alone does
not establish persistent, distributed or authorized cross-tenant reuse.

### G10. Continuous conformance, soundness and diagnostic evidence

- [ ] Maintain versioned positive, invalid, legal-unsupported, wrong-code,
      resource-limit and infrastructure outcomes with source/oracle identities.
- [ ] Import or independently implement applicable language/library conformance
      cases, differential generators and fuzzing with minimization and permanent
      regression retention. Audit licensing/attribution of external test inputs.
- [ ] Test cross-feature combinations, not only individual frozen corpora:
      Drop with generics/unwind, macros with privacy/editions, projections with
      regions, async with captures and cfg/features with cache invalidation.
- [ ] Exercise real CLI packaging, artifacts, diagnostics/spans, exit status,
      time/resource limits and all-unused checks at single/multiple workers.

**Next:** build a machine-readable requirement-to-case index and prioritize
soundness/wrong-code and library/project blockers over accumulating small corpus
counts. **Acceptance:** every claimed contract row links to real observed results,
known valid unsupported programs remain gaps, and discovered failures acquire
minimized tests before the corresponding completion claim is restored.

### G11. Representative performance and final compatibility audit

- [ ] Measure reproducible debug/release, cold/no-change/edit, build/check/test
      sequences, selected/full tests and single/concurrent-project workloads.
      Include CPU/span, memory, cache/transfer, instrumentation and service costs.
- [ ] Retain comparable before/after session-weighted suite receipts, inspect
      regressions by command/change, and measure actual server scaling/topology.
      Synthetic graph wins and subset timings are not full-Rust speed evidence.
- [ ] Audit every declared version/edition/target row, native core/alloc/std,
      independent unchanged projects and self-hosting. Publish remaining gaps,
      unknowns, exclusions and evidence coverage before any full-support claim.

**Next:** define the representative project/workload roster now, mark currently
blocked cases honestly, and benchmark accepted slices without generalizing them.
**Acceptance:** correctness gates pass on the native path and the broad benchmark
comparison is reproducible. Self-hosting is necessary evidence for the intended
compiler workflow, not sufficient proof of full Rust support or optimality.

### Parallel execution plan: six bounded workstreams

A swarm is sufficient only when these streams have owners, independent next
artifacts, agreed shared APIs and reviewers, not merely many active agents.
This is a staffing recommendation, not a claim about currently running workers.

1. **Source, editions and expansion (G0/G1/G5):** own source/span/lookup/token
   contracts and source corpora. Start the source-map/import and hygiene inventory.
2. **Ownership and dataflow (G2/G3):** own place/region/drop contracts and cleanup
   acceptance. Start Drop-order fixtures and the aggregate/SCC region gap map.
3. **Abstraction and typed lowering (G2/G4):** own trait normalization, inference,
   instance/capture contracts. Start associated-type and Iterator-like cases.
4. **Platform and libraries (G6/G7):** own ABI/runtime and pinned core blockers.
   Start the C ABI matrix and library language-item/intrinsic inventory.
5. **Projects and secure reuse (G8/G9):** own manifests, host/target orchestration,
   cache/input/isolation contracts. Start the workspace fixture and invalidation
   threat matrix, coordinating cloud policy in its separate repository.
6. **Independent conformance and measurement (G0/G10/G11):** own frozen oracle
   classification, cross-feature reviews, real-project roster and benchmark
   receipts. Start the traceability index and minimize the highest-risk gaps.

Use explicit file ownership and coordinator-reviewed shared IR/API changes,
especially in the common frontend, type world, CFG and ownership analysis. Do
not let several agents concurrently rewrite those files without an agreed merge
boundary. Four streams are possible by combining source with project discovery
and platform with measurement, but retain an independent correctness reviewer.
The six streams expose useful parallel work now, not six independent full
compiler implementations. Reassess staffing against ready tasks, blocked APIs
and measured integration throughput after each accepted slice.

## Active work and evidence limits

The source-type-alias checkpoint passes all 1,649 hosted tests in 86 binaries
(`20261017-043356389-4183342`), all-target checking
(`20261017-043354163-4183206`), strict all-target lint
(`20261017-043405146-4184072`) and formatting. Its independent executable stock
corpus classifies 31 supported valid, 21 invalid and 10 legal unsupported sources
before 248 native CHECK/BUILD cells at workers 1 and 8. Fixed/explicit constructor
and pattern arguments retain canonical target constraints, while omitted generic
alias argument inference remains unsupported. Two graph tests check native image
identity and unused invalid aliases, and an external-file test exercises generic
alias definition context through imports. The unchanged historical visibility and
match alias sources are promoted after exact executable stock/native acceptance.
An iterative dependency schedule fixes the discovered stack overflow without
increasing thread stacks: a 40-link alias chain succeeds on 128 KiB and growing
cycles fail diagnostically. TYPE-ALIASES.md defines the bounded contract. Full
source organization, abstraction, libraries, projects and full Rust remain open.

The restricted-visibility checkpoint passes all 1,641 hosted tests in 84 binaries
(`20261017-041943076-4105659`), all-target checking
(`20261017-041939306-4105527`), strict all-target lint
(`20261017-041953248-4107714`) and formatting. Its executable stock corpus has
25 supported valid sources, 43 invalid sources and one explicitly legal but
unsupported type alias. All 69 sources run native CHECK/BUILD at workers 1 and 8.
Three graph tests check canonical image identity, nested trait implementation
owner scope and unused invalid declarations. A filesystem stock/native test
covers restricted module/type/field access across real external files. Ordinary
test-thread stack checks accept module depth 128 and reject 129/300. Scope
clone budgets and path normalization bounds have direct regressions. Both
unchanged historical restricted-visibility sources are promoted only after
exact-source stock/native execution. VISIBILITY.md defines the bounded contract.

The named public reexport checkpoint passes all 1,630 hosted tests in 82
binaries (`20261017-040632890-4019165`), all-target checking
(`20261017-040631810-4019111`), strict all-target lint
(`20261017-040637221-4019712`) and formatting. The 44-source independent
stock oracle distinguishes 15 valid from 29 invalid sources. Fourteen valid
sources execute natively and the legal imported-main boundary stays explicitly
unsupported. All 44 are checked in native CHECK/BUILD at workers 1 and 8.
Two graph tests check canonical executable identity and unused private
reexports, and filesystem acceptance covers public aliases from a private
external module. Both unchanged historical public-use sources are promoted
after exact-source stock/native execution. REEXPORTS.md defines the bounded
contract, not full source organization or full Rust support.

The grouped-import checkpoint passes all 1,622 hosted tests in 81 binaries
(`20261017-035808751-3962286`), all-target checking
(`20261017-035807953-3962199`), strict all-target lint
(`20261017-035813817-3962848`) and formatting. Independent stock/native fixtures
cover type-only self aliases, empty groups, trait scopes, namespaces, privacy,
unused imports and cycles. Two graph tests check canonical native image equality
and all-unused rejection. Filesystem tests cover nested external modules around
use-group braces. The two unchanged historical grouped sources now execute
natively after separate exact-source stock execution. Normal-stack resource
tests exercise per-tree and global expansion limits. GROUPED-IMPORTS.md defines
the bounded contract. Named reexports are the following delivered slice above.
Source maps, glob resolution, anonymous imports, full source organization and
full Rust stay open. Restricted visibility is the later bounded slice above.

The full requested outcome is not reached. The native float and labeled-control
checkpoint passes all 1,576 hosted tests (`20261017-031332671-3543206`), all-target
checking (`20261017-031201245-3527437`), strict all-target lint
(`20261017-031343109-3542975`) and formatting. The executable float and label
corpora compare stock and native output at workers 1 and 8. Five float and three
label graph tests additionally check shared facts, malformed IR, contextual
break values, moves, loans and unused invalid bodies. Normal-stack parser nesting
acceptance passes after isolating labeled-expression parsing into a small frame
(`20261017-031112594-3517995`). `FLOATS.md` and `LABELED-CONTROL-FLOW.md` define
these bounded contracts. The subsequent source-default checkpoint passes all
1,608 hosted tests (`20261017-033939643-3821254`), all-target check
(`20261017-033936448-3820907`), strict all-target lint
(`20261017-033944415-3822910`) and formatting. `TRAIT-DEFAULTS.md` defines
universally checked source default bodies and override-aware dispatch. Independent
review exposed potentially-unsized owned Self positions. Executable signatures
and shared IR owned-value gates now require real Sized proof, while borrowed
places retain their original authority. Invalid unused and overridden defaults,
including diverging/deferred Self locals, reject against the stock oracle.
Canonical catalog placeholders remain explicitly unavailable. No category-wide
or full Rust completion is claimed.

Earlier bounded generic inference passes 1,270
hosted tests, strict all-target lint, its unchanged 28 exact source cases and four
instance/trust graph tests. One historical generic-enum source is promoted only
after unchanged stock/native runtime parity. The tracked 27-corpus campaign
verifies 956 native cases on revision 6cef2b5c6feb16ea30692d94b0edb759fd2d91d2,
including 577 accepted parity, 35 declared traps, 266 invalid rejections,
77 legal boundaries and one distinct bounded-instantiation resource outcome.
All raw receipts and the actual 96-worker example are retained. There is no successful
core/alloc/std build, broad existing-project acceptance or representative full-Rust
performance comparison. INTEGER-TYPES.md now records delivered signed/unsigned
primitives through64 bits, typed literals/patterns and existing checked operations.
Integer casts under INTEGER-CASTS.md now pass exact32 stock/native cases and
four graph tests, with soft source inference grounded in primary semantics.
The tracked22corpus native campaign verifies all816cases.
Additional operators remain separate follow-through work.
Full Rust completion remains the goal, not this bounded milestone.
Native Integer128 is delivered under INTEGER128.md. Broader float library/ABI
behavior, broader inference/traits/methods, Drop,
reference-containing values, libraries and project integration remain open.

## Delivered milestones, newest first

Numbers below are evidence at each checkpoint, not cumulative full-Rust coverage.

### Native i128/u128 scalars

INTEGER128.md adds canonical two-limb integer values, checked native arithmetic
including multiplication/division/remainder, all 144 integer casts and direct
f32/f64 wide conversions. Private 16-byte scalar Copy transport preserves both
limbs through generic calls, references and mixed aggregates. Mutable-reference
indexed compounds retain stock RHS/index/current-carrier order, initialization
and every-hop mutable authority, including dead malformed metadata. Physical
scratch planning preserves sealed nominal Copy facts instead of re-proving them
with standalone facts.

All 1,677 hosted tests across 89 binaries, all-target check, strict lint and
formatting pass. Independent stock classification covers 92 exact sources, then
368 native workers1/8 CHECK/BUILD cells with ELF/runtime and artifact gates.
Five unchanged historical integer-type/operator/cast boundaries are promoted
only after executable stock/native parity. Three canonical graph tests, native
arithmetic and roughly 25,000 IEEE conversion oracles complement source coverage.
Retained receipts are data/runs/xps14/2026-10-17-integer128.jsonl. No platform ABI,
profile-exact panic/unwind, complete numeric libraries, full Rust or speed claim
is implied.

### Bounded first-order generic inference

Frontend-only inference now resolves fully omitted call/constructor arguments
from hard expected types and bounded known evidence, with schema-only slots and
rigid caller types even during same-binder recursion. Partial product evidence
retains opaque/numeric leaves privately, defers fallback until all constraints
and never lets unknown evidence force a default. Fallible no-fallback peer hints
are not committed solutions. The ordinary checker still elaborates each child
exactly once in source order before unchanged universal/concrete trust gates.
All 1,270 hosted tests, strict all-target lint and formatting pass. The exact28
frozen source corpus and four graph gates pass, and the real eight-worker native
example executes successfully. Actual reviews closed schema scratch ordering
and constructor lookup-work gaps. No generic references, partial type holes,
global future-use inference, unresolved nested omission or full Rust claim is
introduced. See GENERIC-INFERENCE.md and its retained validation profiles.
Earlier boundary statements describe that historical slice; the newest contracts
and the remaining implementation checklist determine current support.

Integer casts under `INTEGER-CASTS.md` cover all 100 primitive pairs without
overflow traps. Soft source expectations preserve concrete types and checked
literal diagnostics. All 1,067 hosted tests, strict all-target lint, formatting,
exact32 stock/native and four graph tests pass. The unchanged historical cast
fixture and real native example execute. Admitted120cast chains pass native
lowering/execution on the normal stack, with over-cap and cyclic private IR
rejected by bounded lowering. All816tracked22corpus outcomes pass, with23raw
corpus/example receipts retained and the96worker example executed.

Integer foundation through 64 bits is implemented under `INTEGER-TYPES.md`.
All ten integer types preserve exact identity, canonical typed constants and
patterns, contextual radix literals and signed minima. The width-driven native
kernel checks overflow, division and remainder while preserving the private
eight-byte scalar ABI. All 1,021 hosted tests, strict all-target lint and formatting
pass. Exact 32 stock/native fixtures and four graph tests pass, including nine
explicit runtime-101 cases. All 784 tracked twenty-one-corpus native outcomes
pass, with 22 raw corpus/example receipts retained. Casts and the other numeric
gaps remain open.

Mixed-capability reference-result inputs are implemented under
`MIXED-REFERENCE-RETURNS.md`. Shared outputs derive from shared or mutable input
paths, mutable outputs only from mutable inputs, and unrelated inputs retain
their own kinds and region dependencies. Genuine reborrow/unique-return semantics
and all existing limits remain unchanged. All 975 hosted tests, strict all-target
lint, formatting, exact stock20/native20 and four graph tests pass. Four historical
sources execute unchanged after capability promotion. All 752 tracked twenty-corpus
native cases pass, with raw receipts and the executed 96-worker example retained.
Full Rust remains open.

Builtin reference field access is implemented under `REFERENCE-FIELDS.md`.
Frontend normalization inserts actual Deref for `r.field`/`r.0` across reads,
places and scalar hints, preserving original receiver, authority and privacy.
Typed guards keep dynamic reference indexing and enum downcasts unsupported.
All 942 hosted tests, strict all-target lint, formatting and both independent
actual-code reviews pass. The new 20-case stock/CLI corpus and four graph tests
pass. Three historical implicit getter sources and the historical tuple-reference
fixture are promoted with exact source/output bytes unchanged. All 732 tracked
nineteen-corpus native cases pass, with raw receipts and the executed 96-worker
example retained. Full Rust remains open.

Free-function lifetime elision is implemented under `LIFETIME-ELISION.md`.
Stock E0106 probes corrected the initial distinct-ID proposal before release:
output elision counts input reference lifetime positions, not distinct IDs. Fresh
anonymous inputs remain rigid, explicit outputs cannot infer missing bounds, and
NoReference call contracts retain their erased B2 behavior. All 915 hosted tests,
strict all-target lint, formatting and both independent actual-code reviews pass.
The corrected 20-case stock/CLI corpus and four new graph tests pass. Four
historical CLI sources and one graph source are promoted without source edits.
Tracked run `compiler-lifetime-elision-20261016-0915` at `0fe4b3356` passed all
712 cases across eighteen corpora: 425 accepted, 213 invalid rejected, nine
declared runtime-101 traps and 65 legal-unsupported boundaries. All raw receipts
and the executed 96-worker example are retained. Full Rust remains open.

Projected input reference returns are implemented under
`PROJECTED-REFERENCE-RETURNS.md`: complete declaration-derived typed static paths,
source-specific candidate/argument correlation, private-field privacy separation
and bounded occurrence enumeration. All 888 hosted tests, the frozen 30-case stock
corpus and four workers1/8 graph acceptance tests pass. Both independent actual-code
soundness/resource reviews, strict all-target lint and formatting pass. Tracked
M8a run `compiler-projected-reference-returns-20261016-0859` at `9048f4a33` passed
all 692 cases across seventeen corpora: 411 accepted, 205 invalid rejected, nine
declared runtime-101 traps and 67 legal-unsupported boundaries. Raw receipts and
the executed private-field 96-worker example are retained. Full Rust is open.

Mutable whole-parameter reference returns are implemented under
`MUTABLE-REFERENCE-RETURNS.md`, with exact kind contracts, one unique returned
handle, candidate-specific authority and transitive lifetime dependencies. Normal
returns prove universal provenance before consuming a mutable carrier. All 844
hosted tests, strict all-target lint, formatting and both independent actual-code
reviews pass. The exact 30-case stock corpus and four graph tests pass, including
parent freeze/resume and failed unused-callee publication. Tracked M8a run
`compiler-mutable-reference-returns-20261016-0843` at `49a1ae343` passed all 662
cases across sixteen corpora: 394 accepted, 196 invalid rejected, nine declared
runtime-101 traps and 63 legal-unsupported boundaries. All raw receipts and the
96-worker example are retained. Later delivered milestones above add static
projected/mixed-kind results. General lifetime inference and full Rust remain open.

Noescape reference parameters and expected-site reborrows are implemented under
`REFERENCE-CALLS.md`. All 748 hosted tests, strict all-target lint, formatting and
independent resource/soundness review pass. The unchanged 30-case stock corpus
separates 18 successful, eight invalid and four legal-unsupported programs.
Formal referent roots outlive carrier slots, and mutable calls invalidate all
candidate referent proofs without inventing initialization. Tracked fourteen-
corpus native verification passed all 602 cases: 361 successful parity cases,
178 invalid rejections, nine declared runtime-101 traps and 54 explicit legal-
unsupported boundaries. Raw receipts and the executed 96-worker example are
retained under `experiments/results/20261016-m8a-reference-calls-*`.

Shared whole-parameter reference returns are implemented under
`REFERENCE-RETURNS.md`. The frozen 30-case corpus and four retained graph tests
pass hosted acceptance after fixing named-lifetime scanning in filesystem module
loading. The final frozen hosted suite passes 798 tests, strict all-target lint,
formatting and bounded resource/provenance review. Tracked fifteen-corpus native
verification passed all 632 cases at bb3bc5613: 378 successful parity cases, 186
invalid rejections, nine declared runtime-101 traps and 59 legal-unsupported
boundaries. Raw receipts and the executed 96-worker example are retained under
`experiments/results/20261016-m8a-reference-returns-*`. Mutable and projected
results are delivered by later milestones above; general regions and full Rust
remain open.

Genuine nonescaping local references are implemented under `BORROWING.md`, with
canonical issuance-time loan origins, explicit reborrows, backward carrier
liveness and native thin-pointer addressing. The frozen 35-case external oracle
contains 17 accepted programs, 14 invalid programs and four later-stage boundaries.
All 701 hosted tests, strict all-target lint, formatting and final peak-memory
accounting review pass. Tracked M8a validation passed 572 cases across thirteen
corpora: 343 accepted parity, 170 invalid rejections, nine declared runtime-101
cases and 50 legal-unsupported boundaries. Raw receipts and the successful
96-worker native borrowing example are retained. That B1 checkpoint left
reference parameters and implicit expected-site reborrows for the B2 slice above.
General escaping-reference behavior, general regions and reference-containing
aggregates remain open. Later milestones above add bounded input-path reference
results. This does not close the ownership or full-Rust checklist.

Genuine unsigned 64-bit `usize` and runtime array indexing now pass 647 hosted
tests and strict all-target lint. `USIZE.md` records ordered index/readiness
operations, owning RHS snapshots, full-width arithmetic/bounds, direct ignored
non-Copy indexing and conservative selection-proof invalidation. The dedicated
32-case corpus distinguishes 15 successful programs, six invalid programs, nine
explicit exit-101 runtime cases and two legal-unsupported boundaries. The exact
prior array runtime-index boundary is promoted unchanged to accepted. Backward
numeric inference, chained dynamic places, slices, broader primitives and
general references/borrowing remain open. Tracked M8a validation passed 537 cases across
twelve corpora: 326 accepted parity, 156 invalid rejections, nine declared runtime
exit-101 cases and 46 explicit legal-unsupported boundaries. The full-width native
example executed at 96 workers with complete capture; raw receipts are retained.

Fixed arrays and decimal literal indexing now pass 591 hosted tests and strict
all-target lint under `ARRAYS.md`. The bound is 4096 elements, with independent
expanded-path, work and frame limits. Repeats evaluate once even at length zero,
indexed wildcards inspect initialized array storage, and non-Copy indexed moves
reject. The independent 30-case corpus separates 12 supported, 13 invalid and
five legal-unsupported programs. Dynamic `usize` indices, slices and array
patterns remain later work. The tracked M8a run passed505 cases across eleven
corpora (310accepted,150invalid,45legalunsupported), with a successful96-worker
native example and all raw receipts retained. Full Rust completion remains open.

Payload enums now pass 533 hosted tests plus strict all-target lint.
`PAYLOAD-ENUMS.md` defines tuple/braced/unit construction, selected payload
patterns, typed downcasts and separate must-selected-variant proofs. The exact
historical payload fixture is promoted unchanged to accepted. The independent
44-case corpus separates 23 accepted programs, 15 invalid programs and six
legal-unsupported boundaries. The tracked M8a run passed 475 fixtures across
ten corpora: 298 accepted parity cases, 137 invalid rejections and 40 explicit
legal-unsupported boundaries, plus a successful 96-worker native example.
Full Rust remains open, including arrays/numerics,
general patterns, references/borrows/destruction, abstractions and project support.

Typed deferred initialization now passes 465 hosted tests, including branch/match
first writes, loop assignment history, fresh-shadow reads and graph failure
analysis retention. `DEFERRED.md` records the contract. The exact historical
wildcard/uninitialized match fixture is now accepted rather than unsupported.
The dedicated differential corpus contains 10 accepted cases, seven invalid
programs and three explicit inference/pattern boundaries. Payload enums are
the subsequent implemented slice above. The M8a run
passed 431 fixtures across nine corpora: 274 accepted parity cases, 122 invalid
rejections and 35 explicitly unsupported legal programs, with raw receipts retained.

Fieldless enums and scalar/enum match now pass 434 hosted tests. `ENUMS.md`
records nonconsuming variant inspection, selected binding moves, finite coverage
and static ownership checks for unreachable arms. Payload variants, sum-aware
move paths, guards, or/range/nested refutable patterns and broader enum features
remain open. The 59-case independent corpus separates 26 accepted cases,
20 invalid programs and 13 legal-unsupported boundaries. The full M8a regression
run passed 411 fixtures: 263 accepted parity cases, 115 invalid rejections and
33 legal-unsupported boundaries, with raw receipts retained.

Named nonempty structs now extend the cached initialization foundation with
nominal identity, field privacy, whole/partial moves and native aggregate calls.
The following slices remain open: general patterns and destructuring assignment,
references, loans, lifetimes, destructors, enums, broader types and categories above.
This milestone passed 278 hosted tests and all 295 M8a source fixtures, including
19 accepted nominal execution cases, 20 invalid nominal programs and two explicit
legal-unsupported nominal boundaries.

Nested irrefutable let patterns now pass 356 hosted tests, including independent
source acceptance and rejected-move graph retention, plus 352 M8a differential
fixtures: 237 accepted parity cases, 95 invalid rejections and 20 explicitly
legal-unsupported boundaries. Pattern matching/exhaustiveness,
remaining types/control flow and references/loans/destruction remain following
slices. `PATTERNS.md` records the exact implemented contract and explicit gaps.

The internal initialization/move-analysis foundation passed 209 hosted tests and
all 254 existing M8a source fixtures. Both check and emission require the same
cached analysis fact, with resource exhaustion failing closed. This is not a
borrow checker or full Rust support.

Explicit CFG lowering passed 178 hosted tests and 254 M8a differential fixtures:
186 accepted parity cases, 55 invalid rejections and 13 legal-unsupported boundaries.
The old structured native emitter is removed. Both roots share the exact validated
CFG, retained for inspection and explicit trace output.

Copy tuple values passed 169 hosted tests, 38 M8a tuple differential cases and
the existing 28-case Never corpus. These are completed prerequisite slices, not
completion of general types, ownership, source organization or full Rust.

Loop control flow passed 148 hosted tests and the charged M8a differential corpus:
64 generated valid loop programs plus 28 Never-typing cases. Three stock-legal
pure-Never comparison cases remain explicit unsupported inference boundaries.
Labels, for loops and general while expressions remain following work.

External source loading and scalar typed IR are completed prerequisite slices,
not completion of source organization, general types, ownership or full Rust.

No checkbox for an entire category is closed by implementing one slice. Unsupported
legal programs remain tracked as gaps, never counted as conformance successes.

### Eager bitwise checkpoint

`BITWISE.md` is implemented for all ten integer primitives and Bool. Exact types,
eager left-to-right snapshots, canonical normalized results and short-circuit
And/Or separation are validated. All1100 hosted tests and strict all-target lint
pass, including exact20 stock/native cases and four graph tests. Independent
23corpus native follow-through passes all836cases, with24raw receipts and
actual96worker example execution retained. Shifts and compound assignments
were separate work at that checkpoint, and full Rust remains open.

### Checked heterogeneous shifts checkpoint

`SHIFTS.md` is implemented: independently typed integer counts, full-width guard
before hardware masking, truncating left shifts and signed/unsigned right shifts.
The private bounded typed count diagnostic preserves runtime effects, distinguishes
semantic Never from typed noncompletion and shares memo/work/depth limits.
All1145 hosted tests, exact32 stock/native cases, four graph tests and strict
all-target lint pass. Earlier CHECK diagnostics than stock metadata are explicit
for two static invalid fixtures. Independent24corpus evidence passes868cases,
with796normal worker comparisons,54declared trap runs,128generated exit checks
and actual96worker example execution retained in25raw receipts.
Compound assignments, general consteval and full Rust remain open.

### Builtin compound assignment checkpoint

`COMPOUND-ASSIGNMENTS.md` is implemented for the ten primitive operators in
statement contexts. RHS snapshots precede current place/carrier/index selection,
old initialized scalar reads and same-address writes. Dynamic IndexAssignOp
preserves one checked address across division/multiply/shift kernels. Static
updates reuse existing Binary read/write phases, and compound counts share the
same per-function bounded diagnostic evaluator as ordinary shifts. All1191
hosted tests, strict all-target lint, exact32 stock/native plus four graph gates
and the actual real native example pass. Two unchanged historical boundaries
were promoted after native execution. Independent25corpus receipts pass900cases,
including832normal worker comparisons,70declared trap runs,128generated exit
sets and actual96worker example execution, retained in26raw receipts. Overloaded
assignments, full Rust and performance acceptance stay open.

### Shared byte slice (`&[u8]`) native lowering checkpoint

`ArrayToSlice`, `SliceLen` and `SliceIndex` lower through CFG validation,
ownership/borrow analysis and the native backend for nongeneric shared `&[u8]`
parameters and locals only. The slice handle inherits the source reference's
origins, and indexing reads the referent. The native carrier is two words
(data, len), passed as two argument words, and index >= len traps before the load.
The bounded slice-contract runtime runner passes with stock and native output
matching (positive plus three bounds-trap fixtures, workers 1 and 8). Adapted Adler2
acceptance (`--skip ff_44417`) passes 15 of 16 vectors natively at workers 1 and 8,
exactly matching the upstream crate and zlib checksums. These include the 5551..5553 and
22207..22209 NMAX boundaries and all three split vectors. That needed the literal array limit
raised from 4096 to 65535 and saturating recipe footprints, so oversized shapes become
inventoried intrinsic limits instead of hard errors. The 44417-byte vector is
blocked by an ownership analysis resource limit (per-element move paths for
large scalar arrays). Slice returns, `&mut [T]`, non-u8 elements,
range indexing, `for` loops and slice patterns remain unsupported. This is not
unchanged Adler2 acceptance or full Rust support.

### Configured source preparation checkpoint

`SOURCE-PREPARATION.md` records the delivered bounded source envelope: shared
text/file SOURCE facts, cfg-before-child-IO, explicit attribute policy,
original syntax locations and resource/ordinary-stack guards. Hosted all-target
check, 1731 tests in 90 binaries and strict lint pass, alongside 86 stock
fixtures and 344 native worker/mode cells. Full syntax/macros, semantic spans,
library roots, core/alloc/std and full Rust acceptance remain open.

### Source-locked identity runtime checkpoint

`CORE-IDENTITY.md` delivers one unchanged attributed function plus authored
clients, strict full-file/range provenance and scoped inert metadata. All 1758
hosted tests in 93 binaries, strict lint, 72 native identity cells and 344 source
configuration cells pass. Exact-source stock internal-attribute rejections are
recorded separately from installed-core behavioral references. The first native
64/72 and first full 1755/1758 failures remain retained: root-borrow replay was
repaired without demoting valid clients or existing nested-reference fixtures.
Constness does not imply CTFE. Full library crates, macro expansion and full
Rust remain open.
