# Full Rust implementation tracker

The requested outcome is full Rust support, not completion of a sequence of toy
subsets. This checklist stays open until compatibility evidence supports closing
it. `RUST-COMPATIBILITY.md` defines the contract and dependency ordering.

## Completed foundations, not full compatibility

- [x] Independent graph runtime with shared immutable facts and lifecycle telemetry.
- [x] Direct native Linux x86-64 executable generation for the initial subset.
- [x] Inline modules, qualified calls, simple imports and namespace/privacy checks.
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

External source modules: load `mod name;` from `name.rs` or `name/mod.rs`, including
nested inline/file module directory rules. Preserve module privacy and checking of
unused modules. Reject missing/ambiguous/cyclic inputs explicitly and bound source
loading. Expose source loading as graph operation 107 with captured failures and
source-file/byte accounting. An initial whole-tree loading node is an explicitly
coarse implementation, not the intended final per-file graph architecture.

No checkbox for an entire category is closed by implementing one slice. Unsupported
legal programs remain tracked as gaps, never counted as conformance successes.
