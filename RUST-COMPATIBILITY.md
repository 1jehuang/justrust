# Rust compatibility implementation plan

Goal: an independent implementation of the entire Rust language and the
source-to-test-result workflow for existing Rust projects. The first executable
subset is a bootstrap milestone, not the intended final language or architecture.
Normal Rust tools may build our implementation and act as external test oracles.
They must not compile user programs on our native implementation path.

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

## Current slice: inline modules and item resolution

Implement nested `mod name { ... }`, ordinary `pub` on modules/functions,
`crate::`, `self::`, `super::` and relative qualified function paths, plus
module-level simple `use path;` and `use path as alias;` for modules/functions.
Resolve forward declarations and imports, distinguish duplicate declaration
names from legal same-name functions in separate modules, enforce privacy and
local value shadowing, and retain a root-only entry function identity.

For this slice, reject external `mod name;`, grouped/glob imports, `pub use`,
restricted `pub(...)`, block-local imports/items and unsupported namespaces.
Do not silently ignore their semantics. Existing expression/type restrictions
remain in COMPILER.md. External file loading is the following source-organization
slice, not included by describing this one as module support.

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
