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
- [x] Hosted acceptance, stock differential cases and qualified benchmark receipts.

## Remaining implementation program

- [ ] Source organization: complete source maps, grouped/glob imports,
      reexports, restricted visibility, item identities and edition behavior.
      Bounded external module loading is implemented, not the whole category.
- [ ] Typed IR and data: all numeric primitives/literals, tuples, arrays, slices,
      structs/enums, layouts, construction, projections, casts and coercions.
- [ ] Patterns and control flow: destructuring, exhaustiveness, match, loop,
      break/continue, labels, diverging expressions and correct expression typing.
- [ ] Ownership: places, initialization/moves, references, loans, lifetimes,
      borrow/drop checking and destruction.
- [ ] Abstraction: generics, inference, traits, impls/methods, associated items,
      coherence, trait objects and monomorphization.
- [ ] Closures, capture semantics, iterators, async/await and coroutine lowering.
- [ ] Compile-time work: const evaluation/generics, statics, attributes/cfg,
      declarative macro hygiene and procedural macros.
- [ ] Runtime/platform: ABI/FFI, linking, unsafe operations, allocation, intrinsics,
      atomics, TLS, unwinding, debug information and additional targets.
- [ ] Libraries: compile core/alloc/std and external dependencies on the native
      implementation path without delegating compilation to stock tools.
- [ ] Existing projects: manifests, dependency resolution, features, build scripts,
      native libraries, proc-macro integration and test/bench harnesses.
- [ ] Integrated graph: finer source/semantic operations, cache keys, cross-build
      reuse, secure multitenant reuse, resource-aware scheduling and observability.
- [ ] Conformance: pinned positive/negative differential suites, minimized
      soundness/wrong-code regressions, diagnostic tests and real-project gates.
- [ ] Performance: representative debug/release/cold/edit workloads, scaling and
      contention measurements, without equating subset timings with full Rust.
- [ ] Completion: validate the explicit edition/version/target contract, standard
      libraries, real projects and self-hosting. Self-hosting alone is not enough.

## Active work and evidence limits

The full requested outcome is not reached. Latest hosted evidence is 1,067 tests
and 816 independently verified native cases for bounded Linux x86-64 features. There is no successful
core/alloc/std build, broad existing-project acceptance or representative full-Rust
performance comparison. INTEGER-TYPES.md now records delivered signed/unsigned
primitives through64 bits, typed literals/patterns and existing checked operations.
Integer casts under INTEGER-CASTS.md now pass exact32 stock/native cases and
four graph tests, with soft source inference grounded in primary semantics.
The tracked22corpus native campaign verifies all816cases.
Additional operators remain separate follow-through work.
Full Rust completion remains the goal, not this bounded milestone.
Integer128, floats, generics/traits/methods, Drop,
reference-containing values, libraries and project integration remain open.

## Delivered milestones, newest first

Numbers below are evidence at each checkpoint, not cumulative full-Rust coverage.
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
for two static invalid fixtures. Independent24corpus evidence is pending.
Compound assignments, general consteval and full Rust remain open.
