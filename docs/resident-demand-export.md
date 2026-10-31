# Experimental demand exports: implementation boundary

## Frozen decision at v36

Investigation baseline: `a59351f652c9b266b7f29734f2218cbb19a9aaad` (accepted v36).
This is a source-inspection result, not a native acceptance result. No compiler,
test, or workload was run for this investigation. No speedup is claimed.

**No demand-export mode is implemented or enabled.** Both inspected source files
remain byte-identical to that baseline. `RA_JIT_DEMAND_STD` is unchanged. The
reported 6,998 demand-std versus 3,990 default roots motivate investigation but
are not measurements performed here. Reducing the root count alone would not
establish an optimization or external correctness.

The assigned scope permits changes only in `hir-ty-codegen/mod.rs`,
`ra-jit/src/main.rs`, and this document. Within that scope the inspected native
module declarations do not supply a completeness proof for deleting language
exports. An explicit experimental switch would not repair the missing proof.
A switch that always restores every eager root would not be a useful candidate.

## What the existing compiler actually exposes

Locations below refer to the frozen baseline, not upstream APIs.

* `ra-jit/src/main.rs:5276-5296` includes the selected library crates in the
  existing demand-std path. At `5318-5322`, root partitioning deliberately keeps
  exported functions eager, together with main and every no-main root.
* `hir-ty-codegen/mod.rs:1081-1087` emits allocator entries and kept/offered
  statics before the root loop. At `1130-1174`, the root loop drains `cg.wanted`,
  runs `define_waves`, drains newly wanted roots again, and repeats until stable.
  This is the appropriate existing work queue, not a reason to add an unrelated
  recursive compiler or infer demand from source-text names.
* `mod.rs:5526-5537` reads actual native function and data declarations for
  inventory. `ModuleDeclarations::get_functions()` exposes names, linkages and
  signatures. The inventory also captures data declarations. These are genuine
  native declarations, but describe this module, not all objects in the link.
* `mod.rs:4553-4560` provides `exported_symbol`, which resolves attribute-named
  function exports. `main.rs:955-958` uses it as the exact exported function name.
  A future queue extension can map these names to roots. An ambiguous name must
  not silently choose one root. Function/data namespace collisions and target
  naming transformations also need explicit handling.
* `mod.rs:5883-5915` synthesizes allocator forwarding entries and string-named
  default allocator/error-handler imports. Unlike the rustc allocator path at
  `5923-5945`, this path does not directly push each imported target into
  `wanted`. Moving all exports to on-demand without scanning actual imports can
  therefore lose allocator providers even before considering native callbacks.
* `mod.rs:5650-5689` conservatively discovers reachable requests after failures.
  `Unseen` and `want_all_called` preserve broad crate demand when complete
  traversal is unavailable. An export optimization must not narrow that fallback.
* `mod.rs:5995-6012` can turn an imported declaration into a hidden refusal
  stand-in. Declaration linkage alone is therefore not a definitive record of
  whether a real provider was successfully emitted. Existing refusal, rejection,
  and unstitched-definition state must remain authoritative.

## Why module-import closure is necessary but insufficient

The allocator case illustrates a required closure operation: after the existing
root and instance wave, inspect actual function imports, resolve exact exported
providers, enqueue them through `wanted`, and repeat after those definitions and
all generated data/shim work. Do not decide the fixed point before `define_waves`.
Keep statics and their relocations in the same analysis. Do not replace unknown
imports with fixture-specific symbol exceptions.

However, this proves only outgoing demand from the resident object. It cannot
prove absence of incoming demand from other objects or runtime symbol lookup.
The compiler's `Roots` interface (`mod.rs:825-847`) has no complete native input
set, target startup-object description, or certificate that another component
has checked these edges. `whole` describes Rust crate ownership, not native link
closure. `start.is_some()` and `shared == false` do not imply a closed world.

Concrete later input assembly is visible in the existing source:

* `main.rs:4998-5060` links the resident output, optional prebuilt input, native
  libraries and user link arguments only after compilation. Shared-library export
  selection is a separate operation here and must not be changed accidentally.
* `main.rs:5082-5098` constructs the corresponding own-link command used by the
  daemon and patch path. Arbitrary externally supplied commands also participate
  in `link` at `5823` and onward.
* `ra-jit/src/link.rs:125-176` constructs the actual link line from target startup
  and trailing arguments, `native(db, program, target)`, and user arguments.
  `link.rs` was read only, outside this worker's source ownership.
* `link.rs:483-518` gathers `#[link]` declarations and build-script libraries,
  search paths and arguments. These inputs are not represented by the resident
  `ObjectModule` import table. Archives, whole-archive inputs and platform startup
  objects also require their real selection and visibility semantics, not just
  counting undefined function names in the resident object.

For example, a native object's initializer can call an exported Rust function
without the resident object importing either that callback or the initializer.
A native runtime can look up an export dynamically without an undefined-symbol
relocation naming it. These are counterexamples to treating an empty resident
import table as proof of completeness. They are logical counterexamples, not
claims that a particular accepted fixture exercised either behavior.

Rejecting every unknown resident import and reverting to all eager exports is
safe, but cannot establish a reduced-root candidate for programs that import
ordinary external runtime functions. Allowlisting a few familiar libc symbols
would change the unknown-external-symbol contract without proving callback or
lookup behavior. Conversely, accepting unknown imports on the assumption that
link success proves correctness misses runtime lookup and delayed execution.

## Required implementation boundary for a real candidate

A future implementation needs an authoritative native-demand description shared
between root selection, codegen and actual linker/input selection. The owner of
the native operation should provide that description rather than adding a shell
interceptor. This may require coordinated work beyond the two allowed source
files, including `link.rs` and relevant native artifact/manifest validation.

The description must either prove a closed world for a supported configuration
or explicitly require eager retention. It must account for real target naming,
function and data providers, runtime ABI obligations, native callbacks and
lookup, startup inputs, archives, prebuilt inputs and user/build-script linker
arguments. Preserve project exports unconditionally. Keep shared, no-main,
prebuilt, unknown native inputs, retained-base patches, and unproved platform
cases eager unless their separate completeness conditions are established.
Retain existing unknown-symbol resolution and all existing error behavior.

With that description in place, a default-OFF experimental mode can:

1. Preserve default root selection and the independent demand-std control.
2. Classify eligible language exports by actual crate origin and exact export
   identity, never a fixture-symbol list. Preserve mandatory external ABI roots.
3. Seed actual native/runtime incoming demand and retained static demand.
4. Add exact imported language providers to `cg.wanted` at each existing wave's
   fixed point, including allocator shims and newly materialized static edges.
5. Keep all applicable exports eager if any provider, input or traversal is
   ambiguous or incomplete. Preserve existing rejection and refused paths.
6. Ensure mode and native-demand facts are covered by existing source/toolchain
   and configuration/read validation before allowing cached artifact reuse.
   An inventory receipt is diagnostic evidence, not an authorization to reuse a
   manifest. Never weaken manifest/recovery validation to lower measured cost.

This is an implementation boundary, not a claim that all these requirements
already have available APIs or can be met by the existing declaration table.

## Native acceptance required before any promotion

Root owns the actual native build, full tests and matched v36 measurements.
The entire resident source must be frozen first. A meaningful candidate requires
original-path differential fixtures against an unchanged baseline, including:

* allocation, reallocation, deallocation and allocation-error/runtime imports,
  with default and project global allocators;
* attribute-named and unmangled function/data dependencies, project exports,
  retained statics and their relocations;
* native-to-Rust callback/initializer and runtime-lookup obligations, unknown
  external imports and native-input eager fallback;
* shared/no-main/prebuilt/native-link configurations and ambiguity fallback;
* demanded unsupported code, unresolved native symbols, type errors, failing
  assertions and recovery after errors, preserving full diagnostics/exit status;
* fresh builds, warm edits, manifest hits, process recovery and transitions
  between experimental modes without source edits.

Fixture names must not appear in production demand logic. Compare actual
behavior, symbols/ABI where applicable, refusals, diagnostics and cache
invalidation, not only a smaller compiled-root counter. Record matched timings
and breakdowns for manifest and recovery revalidation only after correctness.
No differential fixture is claimed as implemented or run in this docs-only
outcome.

## Frozen handoff

`ROOT/demand-export-v37/` contains exact HEAD source originals, identical source
candidate bytes, this document's candidate bytes, the docs-only patch and SHA-256
manifests. `FROZEN.md` records the decision and checks. There is no native source
candidate to build, no changed artifact/cache contract, and no release or commit
from this worker. Reopening implementation requires a new freeze coordinated
with the root owner, not editing beneath a running native build.
