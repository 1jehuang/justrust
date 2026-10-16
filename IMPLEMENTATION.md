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
- [x] Hosted acceptance, stock differential cases and qualified benchmark receipts.

## Remaining implementation program

- [ ] Source organization: external modules, source maps, grouped/glob imports,
      reexports, restricted visibility, item identities and edition behavior.
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

## Active slice

Typed deferred initialization now passes 465 hosted tests, including branch/match
first writes, loop assignment history, fresh-shadow reads and graph failure
analysis retention. `DEFERRED.md` records the contract. The exact historical
wildcard/uninitialized match fixture is now accepted rather than unsupported.
The dedicated differential corpus contains 10 accepted cases, seven invalid
programs and three explicit inference/pattern boundaries. Payload enums remain
the next design/implementation slice, not a completed capability. The M8a run
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
