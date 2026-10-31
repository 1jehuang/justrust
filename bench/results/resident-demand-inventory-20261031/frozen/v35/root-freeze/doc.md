# Demand library artifact: v35 native inventory prerequisite

This is the licensed Blaze-derived resident implementation, not the clean-room
compiler. The original v34 boundary report and snapshots remain untouched under
`demand-library-artifact-v34/`. Its baseline was
`9f3bab61eb2d37f75f232525e3f4714579aad791`.

## Stage 1 only, no artifact reuse

v35 exports exact **in-memory, database-local native codegen metadata**. It does
not split an object, select a smaller export set, hydrate a partial library,
write an inventory to disk, introduce a cache key, or claim a speed improvement.
Default root selection, dependency traversal, refusal handling, ABI exports and
linking remain the existing implementation. `RA_JIT_DEMAND_STD=1` retains its
existing meaning and does not enable the new inventory.

Only the exact separate startup opt-in `RA_JIT_DEMAND_INVENTORY=1` enables
inventory collection. It is sampled once when native codegen first constructs
an object. Missing, `0`, `true`, whitespace-padded and all other values are off.
The off path does not enumerate semantic maps, clone payloads, or hash anything.
It adds optional-state checks at the native definition hooks. Root must measure
that overhead before accepting. Existing timing boundaries are retained, with
opt-in snapshot work included in the object-emission phase.

## Actual capture boundary and types

`CodegenUnit::inventory` holds `Option<CodegenInventory>`. `Build` retains it for
the diagnostic summary. Inventory never feeds the compiler, manifest, resolver,
linker or rejection decisions.

The successful full-build round calls `ObjectCodegen::inventory` immediately
before `finish`. Discarded/restarted rounds do not return an inventory. Capture
clones native state and makes no new MIR/inference/dependency queries.

* `RootInventory` copies the exact `Roots` inputs, including entry and on-demand
  `FunctionId`s, start, shared/whole, kept and offered `StaticId`s. It separately
  retains compiled `(FunctionId, symbol)` outcomes and skipped roots with their
  original `CodegenError`. A kept/offered static is a request, not a successful
  definition. An on-demand root is not asserted reachable merely because it was
  available.
* Actual `Instance` and `InstanceDef` types are exported without changing their
  representation or identity semantics. Keys preserve the complete instance
  kind (including closures, coroutines and generated shims), stored generic
  arguments and overflow-check mode. No demangled or mangled-name reconstruction
  is used.
* `instances`, `envs`, `found_instances`, `forwarded`, `pending`, unit instance
  and root maps, unit-tried and ahead state are copied separately. An instance
  declaration is **not** a claim of local compilation. Successfully stitched
  native `FuncId`s and declaration linkage/signatures are separate inventories.
  Unit maps describe their state at capture, not a history of every LLVM import.
* `cg.rejected` is copied as exact `(Instance, StoredParamEnvAndCrate, reason)`
  entries. This is the round's known rejection state, not a filtered list of only
  currently reached rejections. Explicit denied root IDs, hidden stand-in function
  IDs and unstitched declarations are retained too. Skipped root errors and final
  stand-in messages are distinct, preserving their different meanings.
* `statics`, `frozen_statics`, `imported_statics`, `shared_statics`, and actual
  definition order retain their exact `StaticId` to native `DataId` mappings.
  Vtables preserve `StoredTy`, trait identity and stored substitutions. Shared
  constant identities preserve the actual `ConstBlob` (bytes, relocations,
  alignment and stored type-ID fixups), not just an exported name. Successful
  `fill_data` calls also retain the original typed blob before type-ID fixups,
  including non-shared/writable allocations, alongside post-fixup native inputs.

## Anonymous-data boundary, explicitly limited

Every native `ObjectCodegen` call to `Module::define_data` goes through one
helper that makes the original native call first. Only after it succeeds, and
only with capture enabled, it records `(DataId, DataDescription)`. This includes
anonymous/local and writable allocations that never enter the shared-constant
map, named and frozen statics, vtables, weak-pointer data and stand-in
placeholders. The description retains initial bytes or zero initialization,
exact function/data relocation targets and offsets/addends, alignment, custom
section and used status. Native data declarations separately retain name,
linkage, writable and TLS state. A failed definition is not recorded.

Stand-in message replacement and wasm custom sections are retained separately.
This is exact **native module input inventory**, not a complete final-object
inventory or anonymous-allocation ownership graph. Backend finalization may
introduce unwind/debug/object-format metadata after capture, and those bytes
are not claimed to be source-level anonymous-data coverage. Source inspection
found `unwind.rs::add_function` (LSDA) and `unwind.rs::name_personality` data
definitions outside current ownership. Exact `unwind.rs` ownership was requested
for those payloads, not treated as a blocker to this source-level prerequisite.
No ownership or cross-object ABI certificate can be inferred from a `DataId`.

## Successful patches and manifest hits

At this stage successful patch returns have no inventory and an opted-in build
prints that absence explicitly. They must not be mistaken for full-build coverage.
Exact delta capture requires transport through
`hir-ty-codegen/patch.rs::Patch`, whose ownership was explicitly requested from
the coordinator. A patch delta would still not be a merged inventory of the
retained base and all prior patches. `is_patch` reserves that distinction.

A manifest hit compiles nothing, so `Build::of_manifest` has no native inventory.
The new `RA_JIT_*` configuration variable is already separated by the existing
manifest configuration-key rules. No manifest or read-certificate API was
changed and no inventory is persisted with a manifest.

## Source assertions added, not executed by this worker

* Exact opt-in parsing, including rejected alternate/padded values.
* Native data fixture using the actual shared definition helper: capture off/on
  must yield byte-identical objects and identical duplicate-definition errors.
  It asserts successful-only definition coverage, anonymous/local identity,
  writable/TLS metadata, used/alignment and exact function/data relocations.
* Real RA database fixture: distinct source `FunctionId` and `StaticId` requests,
  compiled/skipped root separation, unchanged refusal text and exact kept versus
  offered identity. Instance hash/equality assertions use that actual generic
  function with different interned type substitutions, overflow-check settings,
  function roots and shim kinds. This validates the identity/copy helpers, not
  a claim that a complete program was compiled by the fixture.

Root owns compilation, running these assertions, full codegen-output equality,
refusal/kept/export fixtures, patch and recovery acceptance, and off/on overhead
measurements. No builds, tests, native jobs, installation, commits or pushes were
run by this worker. Formatting and whitespace/source inspection are the only
checks performed here.

## Frozen source handoff

v35 full original/candidate bytes, patch, hash manifest and explicit readiness
record live under:

`/home/jeremy/.jcode/scratch/blaze-native-engine-20261030/demand-library-artifact-v35`

Do not treat source formatting as native validation. This stage removes the
opaque-count API limitation identified in v34 for full native rounds. Correct
split/reuse still needs validated closure ownership, export/refusal contracts,
source/config/discovered-read certificates, deterministic versioned identities
and acceptance/performance gates before any persistent artifact may be used.
