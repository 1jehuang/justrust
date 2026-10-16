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

Named nonempty structs now extend the cached initialization foundation with
nominal identity, field privacy, whole/partial moves and native aggregate calls.
The following slices remain open: destructuring and patterns, references, loans,
lifetimes, destructors, enums, broader types and all remaining categories above.
This milestone passed 278 hosted tests and all 295 M8a source fixtures, including
19 accepted nominal execution cases, 20 invalid nominal programs and two explicit
legal-unsupported nominal boundaries. Next is irrefutable let destructuring with
projected moves and wildcard semantics, not general match or borrowing.

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
